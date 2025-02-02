use async_std::{fs::File, stream, task};
use crossbeam_channel::Sender;
use futures::{
    future::FutureExt,
    prelude::{stream::StreamExt, *},
    select,
};
use serde::{Deserialize, Serialize};
use serde_json::to_string;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};
use structopt::StructOpt;
use tide::{Body, Request, Response, Result};
use time_source::Time;

#[derive(Serialize, Deserialize, Debug, Clone)]
enum FinalizationType {
    IC,
    FP,
    DK,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HeightMetrics {
    latency: Duration,
    fp_finalization: FinalizationType,
}

#[derive(Serialize, Deserialize, Debug)]
struct BenchmarkResult {
    finalization_times: BTreeMap<Height, Option<HeightMetrics>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ArtifactDelayInfo {
    sent: Option<Time>,
    received: Option<Time>,
}

pub mod network_layer;
use crate::{
    consensus_layer::height_index::Height,
    network_layer::Peer,
    time_source::{get_absolute_end_time, system_time_now},
};

pub mod artifact_manager;
pub mod consensus_layer;
pub mod crypto;
pub mod time_source;

#[derive(StructOpt, Debug)]
struct Opt {
    #[structopt(long, default_value = "3")]
    blocksize: usize, // number of bytes we put in a block
    #[structopt(long)]
    r: u8, // replica number
    #[structopt(long, default_value = "6")]
    n: u8, // total number of nodes
    #[structopt(long, default_value = "1")]
    f: u8, // number of byzantine nodes
    #[structopt(long, default_value = "1")]
    p: u8, // number of disagreeing nodes
    #[structopt(long)]
    cod: bool, // enable Fast IC Consensus
    #[structopt(long, default_value = "300")]
    t: u64, // time to run replica
    #[structopt(long, default_value = "500")]
    d: u64, // notary delay
    #[structopt(long, default_value = "56789")]
    port: u64, // port which the peers listen for connections
    #[structopt(name = "broadcast_interval", long, default_value = "100")]
    broadcast_interval: u64, // interval after which artifacts are broadcasted
    #[structopt(
        name = "artifact_manager_polling_interval",
        long,
        default_value = "200"
    )]
    artifact_manager_polling_interval: u64, // periodic duration of `PollEvent` in milliseconds
    #[structopt(name = "broadcast_interval_ramp_up", long, default_value = "200")]
    broadcast_interval_ramp_up: u64, // interval after which artifacts are broadcasted during ramp up in milliseconds
    #[structopt(name = "ramp_up_time", long, default_value = "100")]
    ramp_up_time: u64, // time to ramp up replica in seconds
}

#[derive(Clone)]
pub struct SubnetParams {
    total_nodes_number: u8,
    byzantine_nodes_number: u8,
    disagreeing_nodes_number: u8,
    fast_internet_computer_consensus: bool,
    artifact_delay: u64,
    artifact_manager_polling_interval: u64,
    blocksize: usize,
}

impl SubnetParams {
    fn new(n: u8, f: u8, p: u8, cod: bool, d: u64, pi: u64, blocksize: usize) -> Self {
        Self {
            total_nodes_number: n,
            byzantine_nodes_number: f,
            disagreeing_nodes_number: p,
            fast_internet_computer_consensus: cod,
            artifact_delay: d,
            artifact_manager_polling_interval: pi,
            blocksize,
        }
    }
}

async fn get_local_peer_id(req: Request<String>) -> Result {
    let peer_id = req.state();
    let res = Response::builder(200)
        .header("Content-Type", "application/json")
        .body(Body::from_json(peer_id)?)
        .build();
    Ok(res)
}

async fn post_remote_peers_addresses(
    mut req: Request<String>,
    sender: Arc<RwLock<Sender<String>>>,
) -> Result {
    let addresses = req.body_string().await?;
    sender.write().unwrap().send(addresses).unwrap();
    let res = Response::builder(200)
        .header("Content-Type", "application/json")
        .build();
    Ok(res)
}

#[async_std::main]
async fn main() -> Result<()> {
    let opt = Opt::from_args();
    println!("Replica:{}, blocksize:{}, FICC:{}, f:{}, p:{}, notar_delay:{}, broadcast_interval:{}, and art_man poll interval:{}", opt.r, opt.blocksize, opt.cod, opt.f, opt.p, opt.d, opt.broadcast_interval, opt.artifact_manager_polling_interval);

    let finalizations_times =
        Arc::new(RwLock::new(BTreeMap::<Height, Option<HeightMetrics>>::new()));
    let cloned_finalization_times = Arc::clone(&finalizations_times);

    let mut my_peer = Peer::new(
        opt.r,
        opt.port,
        SubnetParams::new(
            opt.n,
            opt.f,
            opt.p,
            opt.cod,
            opt.d,
            opt.artifact_manager_polling_interval,
            opt.blocksize,
        ),
        "gossip_blocks",
        cloned_finalization_times,
    )
    .await;

    // Listen on all available interfaces at port specified in opt.port
    my_peer.listen_for_dialing();
    let local_peer_id = my_peer.id.to_string();

    let (sender_peers_addresses, receiver_peers_addresses) =
        crossbeam_channel::unbounded::<String>();

    thread::spawn(move || {
        let mut peers_addresses = String::new();
        println!("Waiting to receive peers addresses...");
        if let Ok(addresses) = receiver_peers_addresses.recv() {
            peers_addresses.push_str(&addresses);
        }
        println!("Received peers addresses: {}", peers_addresses);

        task::block_on(async {
            my_peer.dial_peers(peers_addresses);

            let starting_time = system_time_now();
            let relative_duration = Duration::from_millis(opt.t * 1000);
            let absolute_end_time = get_absolute_end_time(starting_time, relative_duration);
            let initation_end_time = starting_time + Duration::from_millis(opt.ramp_up_time * 1000);
            let mut initation_phase = true;
            loop {
                if initation_phase {
                    let mut broadcast_interval =
                        stream::interval(Duration::from_millis(opt.broadcast_interval_ramp_up));
                    select! {
                        _ = broadcast_interval.next().fuse() => {
                            // prevent Mdns expiration event by periodically broadcasting keep alive messages to peers
                            // if any locally generated artifact, broadcast it
                            if my_peer.artifact_manager_started() {
                                my_peer.broadcast_message();
                            }
                        },
                        event = my_peer.get_next_event() => my_peer.match_event(event),
                    }
                    if system_time_now() > initation_end_time {
                        initation_phase = false;
                    }
                } else if system_time_now() < absolute_end_time {
                    let mut broadcast_interval =
                        stream::interval(Duration::from_millis(opt.broadcast_interval));
                    select! {
                        _ = broadcast_interval.next().fuse() => {
                            // prevent Mdns expiration event by periodically broadcasting keep alive messages to peers
                            // if any locally generated artifact, broadcast it
                            if my_peer.artifact_manager_started() {
                                my_peer.broadcast_message();
                            }
                        },
                        event = my_peer.get_next_event() => my_peer.match_event(event),
                    }
                } else {
                    // println!("\nStopped replica");
                    let benchmark_result = BenchmarkResult {
                        finalization_times: finalizations_times.read().unwrap().clone(),
                    };

                    let encoded = to_string(&benchmark_result).unwrap();
                    let mut file = File::create("./benchmark/benchmark_results.json".to_string())
                        .await
                        .unwrap();
                    file.write_all(encoded.as_bytes()).await.unwrap();

                    break;
                }
            }
            std::process::exit(0);
        });
    });

    let mut app = tide::with_state(local_peer_id);

    app.at("/local_peer_id").get(get_local_peer_id);

    let arc_sender_peers_addresses: Arc<RwLock<Sender<String>>> =
        Arc::new(RwLock::new(sender_peers_addresses));
    let cloned_arc_sender_peers_addresses = Arc::clone(&arc_sender_peers_addresses);
    app.at("/remote_peers_addresses").post(move |req| {
        post_remote_peers_addresses(req, Arc::clone(&cloned_arc_sender_peers_addresses))
    });

    app.listen(format!("0.0.0.0:{}", opt.port + 1)).await?;

    Ok(())
}
