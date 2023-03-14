use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock}, time::Duration,
};
use std::thread::sleep;
use crossbeam_channel::Receiver;
use futures::{prelude::stream::StreamExt, stream::SelectNextSome};
use libp2p::{
    floodsub::{Floodsub, FloodsubEvent, Topic},
    identity::Keypair,
    multiaddr::Protocol,
    multihash::Multihash,
    swarm::SwarmEvent,
    NetworkBehaviour, PeerId, Swarm, Multiaddr,
};
use serde::{Deserialize, Serialize};

use crate::{
    artifact_manager::ArtifactProcessorManager,
    consensus_layer::{
        artifacts::{ConsensusMessage, UnvalidatedArtifact},
        height_index::Height,
    },
    time_source::{SysTimeSource, TimeSource, system_time_now},
    SubnetParams, HeightMetrics, crypto::CryptoHash, ArtifactDelayInfo,
};

// We create a custom network behaviour that combines floodsub and mDNS.
// Use the derive to generate delegating NetworkBehaviour impl.
#[derive(NetworkBehaviour)]
#[behaviour(out_event = "OutEvent")]
pub struct P2PBehaviour {
    floodsub: Floodsub,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum OutEvent {
    Floodsub(FloodsubEvent),
}

impl From<FloodsubEvent> for OutEvent {
    fn from(v: FloodsubEvent) -> Self {
        Self::Floodsub(v)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Message {
    ConsensusMessage(ConsensusMessage),
    KeepAliveMessage,
}

pub struct Peer {
    replica_number: u8,
    id: PeerId,
    first_block_delay: u64,
    round: usize,
    rank: u64,
    floodsub_topic: Topic,
    swarm: Swarm<P2PBehaviour>,
    peers_addresses: String,
    subscribed_peers: BTreeSet<PeerId>,
    receiver_outgoing_artifact: Receiver<ConsensusMessage>,
    time_source: Arc<SysTimeSource>,
    manager: ArtifactProcessorManager,
}

impl Peer {
    pub async fn new(
        replica_number: u8,
        peers_addresses: String,
        subnet_params: SubnetParams,
        first_block_delay: u64,
        topic: &str,
        finalization_times: Arc<RwLock<BTreeMap<Height, Option<HeightMetrics>>>>,
    ) -> Self {
        let starting_round = 1;
        // Create a random PeerId
        let local_key = Keypair::generate_ed25519();
        let local_peer_id = PeerId::from(local_key.public());

        // Set up an encrypted DNS-enabled TCP Transport
        let transport = libp2p::development_transport(local_key).await.unwrap();

        // Create a Floodsub topic
        let floodsub_topic = Topic::new(topic);

        // channel used to transmit locally generated artifacts from the consensus layer to the network layer so that they can be broadcasted to other peers
        let (sender_outgoing_artifact, receiver_outgoing_artifact) =
            crossbeam_channel::unbounded::<ConsensusMessage>();

        // Initialize the time source.
        let time_source = Arc::new(SysTimeSource::new());

        // Create a Swarm to manage peers and events
        let local_peer = Self {
            replica_number,
            id: local_peer_id,
            first_block_delay,
            round: starting_round,
            rank: 0, // updated after Peer object is instantiated
            floodsub_topic: floodsub_topic.clone(),
            swarm: {
                let mut behaviour = P2PBehaviour {
                    floodsub: Floodsub::new(local_peer_id),
                };

                behaviour.floodsub.subscribe(floodsub_topic);
                Swarm::new(transport, behaviour, local_peer_id)
            },
            peers_addresses,
            subscribed_peers: BTreeSet::new(),
            receiver_outgoing_artifact,
            time_source: time_source.clone(),
            manager: ArtifactProcessorManager::new(
                replica_number,
                subnet_params,
                time_source,
                sender_outgoing_artifact,
                finalization_times,
            ),
        };
        // println!(
        //     "Local node initialized with number: {} and peer id: {:?}",
        //     local_peer.replica_number, local_peer_id
        // );
        local_peer
    }

    pub fn listen_for_dialing(&mut self) {
        self.swarm
            .listen_on(
                "/ip4/0.0.0.0/tcp/56789"
                    .parse()
                    .expect("can get a local socket"),
            )
            .expect("swarm can be started");
    }

    pub fn broadcast_message(&mut self) {
        match self.receiver_outgoing_artifact.try_recv() {
            Ok(outgoing_artifact) => {
                // println!("Broadcasted locally generated artifact");
                self.swarm.behaviour_mut().floodsub.publish(
                    self.floodsub_topic.clone(),
                    serde_json::to_string::<Message>(&Message::ConsensusMessage(outgoing_artifact))
                        .unwrap(),
                );
            }
            Err(_) => {
                self.swarm.behaviour_mut().floodsub.publish(
                    self.floodsub_topic.clone(),
                    serde_json::to_string::<Message>(&Message::KeepAliveMessage).unwrap(),
                );
            }
        }
    }

    pub fn get_next_event(&mut self) -> SelectNextSome<'_, Swarm<P2PBehaviour>> {
        self.swarm.select_next_some()
    }

    pub fn match_event<T>(&mut self, event: SwarmEvent<OutEvent, T>) {
        match event {
            SwarmEvent::NewListenAddr { mut address, .. } => {
                address.push(Protocol::P2p(
                    Multihash::from_bytes(&self.id.to_bytes()[..]).unwrap(),
                ));
                println!("Listening on {:?}", address);
                if self.replica_number == 1 {
                    for peer_address in self.peers_addresses.split(',') {
                        let remote_peer_multiaddr: Multiaddr = peer_address.parse().expect("valid address");
                        self.swarm.dial(remote_peer_multiaddr.clone()).expect("known peer");
                        println!("Dialed remote peer: {:?}", peer_address);
                        let remote_peer_id = PeerId::try_from_multiaddr(&remote_peer_multiaddr).expect("multiaddress with peer ID");
                        if !self.subscribed_peers.contains(&remote_peer_id) {
                            self.swarm
                                .behaviour_mut()
                                .floodsub
                                .add_node_to_partial_view(remote_peer_id);
                            self.subscribed_peers.insert(remote_peer_id);
                            println!("Added peer with ID: {:?} to broadcast list", remote_peer_id);
                        }
                    }
                }
            }
            SwarmEvent::Behaviour(OutEvent::Floodsub(floodsub_event)) => {
                match floodsub_event {
                    FloodsubEvent::Message(floodsub_message) => {
                        let floodsub_content = String::from_utf8_lossy(&floodsub_message.data);
                        let message =
                            serde_json::from_str::<Message>(&floodsub_content).expect("can parse artifact");
                        self.handle_incoming_message(message);
                    },
                    FloodsubEvent::Subscribed { peer_id: remote_peer_id, .. } => {
                        if !self.subscribed_peers.contains(&remote_peer_id) {
                            if self.replica_number != 1 {
                                self.swarm
                                    .behaviour_mut()
                                    .floodsub
                                    .add_node_to_partial_view(remote_peer_id);
                                self.subscribed_peers.insert(remote_peer_id);
                                println!("Added peer with ID: {:?} to broadcast list", remote_peer_id);
                            }
                        }
                    },
                    _ => println!("Unhandled floodsub event"), 

                }
                
            },
            _ => {
                // println!("Unhandled swarm event");
            }
        }
    }

    pub fn handle_incoming_message(&mut self, message_variant: Message) {
        match message_variant {
            Message::KeepAliveMessage => (),
            Message::ConsensusMessage(consensus_message) => {
                self.manager.on_artifact(
                    UnvalidatedArtifact::new(consensus_message, self.time_source.get_relative_time()),
                );
            }
        }
    }
}
