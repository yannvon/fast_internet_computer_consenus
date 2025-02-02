use crossbeam_channel::{Receiver, Sender};
use futures::{prelude::stream::StreamExt, stream::SelectNextSome};
use libp2p::{
    floodsub::{Floodsub, FloodsubEvent, Topic},
    identity::Keypair,
    multiaddr::Protocol,
    multihash::Multihash,
    swarm::SwarmEvent,
    Multiaddr, NetworkBehaviour, PeerId, Swarm,
};
use serde::{Deserialize, Serialize};
use std::thread::sleep;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock},
    time::Duration,
};

use crate::{
    artifact_manager::ArtifactProcessorManager,
    consensus_layer::{
        artifacts::{ConsensusMessage, UnvalidatedArtifact},
        consensus_subcomponents::{
            block_maker::{Block, Payload},
            notary::NotarizationShareContent,
        },
        height_index::Height,
    },
    crypto::{Hashed, Signed},
    time_source::system_time_now,
    HeightMetrics, SubnetParams,
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
    pub id: PeerId,
    artifact_manager_started: bool,
    subnet_params: SubnetParams,
    floodsub_topic: Topic,
    swarm: Swarm<P2PBehaviour>,
    listening_port: u64,
    subscribed_peers: BTreeSet<PeerId>,
    connected_peers: BTreeSet<PeerId>,
    receiver_outgoing_artifact: Receiver<ConsensusMessage>,
    sender_outgoing_artifact: Sender<ConsensusMessage>,
    finalization_times: Arc<RwLock<BTreeMap<Height, Option<HeightMetrics>>>>,
    manager: Option<ArtifactProcessorManager>,
}

impl Peer {
    pub async fn new(
        replica_number: u8,
        listening_port: u64,
        subnet_params: SubnetParams,
        topic: &str,
        finalization_times: Arc<RwLock<BTreeMap<Height, Option<HeightMetrics>>>>,
    ) -> Self {
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

        // Create a Swarm to manage peers and events
        Self {
            replica_number,
            id: local_peer_id,
            artifact_manager_started: false,
            subnet_params,
            floodsub_topic: floodsub_topic.clone(),
            swarm: {
                let mut behaviour = P2PBehaviour {
                    floodsub: Floodsub::new(local_peer_id),
                };

                behaviour.floodsub.subscribe(floodsub_topic);
                Swarm::new(transport, behaviour, local_peer_id)
            },
            listening_port,
            subscribed_peers: BTreeSet::new(),
            connected_peers: BTreeSet::new(),
            receiver_outgoing_artifact,
            sender_outgoing_artifact,
            finalization_times,
            manager: None,
        }
    }

    pub fn listen_for_dialing(&mut self) {
        self.swarm
            .listen_on(
                format!("/ip4/0.0.0.0/tcp/{}", self.listening_port)
                    .parse()
                    .expect("can get a local socket"),
            )
            .expect("swarm can be started");
    }

    pub fn dial_peers(&mut self, peers_addresses: String) {
        for peer_address in peers_addresses.split(',') {
            let remote_peer_multiaddr: Multiaddr = peer_address.parse().expect("valid address");
            let remote_peer_id = PeerId::try_from_multiaddr(&remote_peer_multiaddr)
                .expect("multiaddress with peer ID");
            if !self.subscribed_peers.contains(&remote_peer_id) {
                self.swarm
                    .dial(remote_peer_multiaddr.clone())
                    .expect("known peer");
                self.swarm
                    .behaviour_mut()
                    .floodsub
                    .add_node_to_partial_view(remote_peer_id);
                self.subscribed_peers.insert(remote_peer_id);
                println!(
                    "Dialed remote peer: {:?} and added to broadcast list",
                    peer_address
                );
            }
        }
    }

    pub fn broadcast_message(&mut self) {
        if let Ok(outgoing_artifact) = self.receiver_outgoing_artifact.try_recv() {
            if self.replica_number == 1 {
                match &outgoing_artifact {
                    ConsensusMessage::BlockProposal(proposal) => {
                        if proposal.content.value.height == 1 {
                            sleep(Duration::from_millis(100));
                        }
                    }
                    ConsensusMessage::NotarizationShare(share) => match &share.content {
                        NotarizationShareContent::COD(ack) => {
                            if ack.height == 1 {
                                println!("Rebroadcasting first block proposal");
                                self.swarm.behaviour_mut().floodsub.publish(
                                    self.floodsub_topic.clone(),
                                    serde_json::to_string::<Message>(&Message::ConsensusMessage(
                                        ConsensusMessage::BlockProposal(Signed {
                                            content: Hashed {
                                                hash: String::from("block1"),
                                                value: Block {
                                                    parent: String::from("block0"),
                                                    payload: Payload::new(
                                                        self.subnet_params.blocksize,
                                                    ),
                                                    height: 1,
                                                    rank: 0,
                                                },
                                            },
                                            signature: self.replica_number,
                                        }),
                                    ))
                                    .unwrap(),
                                );
                                println!("Dupsko");
                            }
                        }
                        NotarizationShareContent::ICC(share) => {
                            if share.height == 1 {
                                println!("Rebroadcasting first block proposal");
                                self.swarm.behaviour_mut().floodsub.publish(
                                    self.floodsub_topic.clone(),
                                    serde_json::to_string::<Message>(&Message::ConsensusMessage(
                                        ConsensusMessage::BlockProposal(Signed {
                                            content: Hashed {
                                                hash: String::from("block1"),
                                                value: Block {
                                                    parent: String::from("block0"),
                                                    payload: Payload::new(
                                                        self.subnet_params.blocksize,
                                                    ),
                                                    height: 1,
                                                    rank: 0,
                                                },
                                            },
                                            signature: self.replica_number,
                                        }),
                                    ))
                                    .unwrap(),
                                );
                            }
                        }
                    },
                    _ => (),
                }
            }
            // println!("\nBroadcasted locally generated artifact: {:?}", outgoing_artifact);
            self.swarm.behaviour_mut().floodsub.publish(
                self.floodsub_topic.clone(),
                serde_json::to_string::<Message>(&Message::ConsensusMessage(outgoing_artifact))
                    .unwrap(),
            );
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
                // println!("Listening on: {:?}", address);
                // println!("Local peer ID: {:?}", self.id);
            }
            SwarmEvent::Behaviour(OutEvent::Floodsub(floodsub_event)) => match floodsub_event {
                FloodsubEvent::Message(floodsub_message) => {
                    let floodsub_content = String::from_utf8_lossy(&floodsub_message.data);
                    let message = serde_json::from_str::<Message>(&floodsub_content)
                        .expect("can parse artifact");
                    self.handle_incoming_message(message);
                }
                FloodsubEvent::Subscribed {
                    peer_id: remote_peer_id,
                    ..
                } => {
                    if !self.subscribed_peers.contains(&remote_peer_id) {
                        self.swarm
                            .behaviour_mut()
                            .floodsub
                            .add_node_to_partial_view(remote_peer_id);
                        self.subscribed_peers.insert(remote_peer_id);
                        println!("Added peer with ID: {:?} to broadcast list", remote_peer_id);
                    }
                }
                _ => println!("Unhandled floodsub event"),
            },
            SwarmEvent::ConnectionEstablished {
                peer_id: remote_peer_id,
                ..
            } => {
                if !self.connected_peers.contains(&remote_peer_id) {
                    println!(
                        "Connection established with remote peer: {:?}",
                        remote_peer_id
                    );
                    self.connected_peers.insert(remote_peer_id);
                    if self.connected_peers.len()
                        == (self.subnet_params.total_nodes_number - 1) as usize
                    {
                        self.manager = Some(ArtifactProcessorManager::new(
                            self.replica_number,
                            self.subnet_params.clone(),
                            self.sender_outgoing_artifact.clone(),
                            Arc::clone(&self.finalization_times),
                        ));
                        println!("\nArtifact manager started");
                        self.artifact_manager_started = true;
                    }
                }
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                println!("Peer: {} disconnected", peer_id)
            }
            SwarmEvent::Dialing(peer_id) => println!("Dialed peer {}", peer_id),
            SwarmEvent::ListenerError { listener_id, .. } => {
                println!("Listener with ID: {:?}", listener_id)
            }
            SwarmEvent::IncomingConnection { .. } => println!("Incoming connection"),
            SwarmEvent::IncomingConnectionError { local_addr, .. } => {
                println!("Incoming connection error: {:?}", local_addr)
            }
            SwarmEvent::ListenerClosed { listener_id, .. } => {
                println!("Listener closed: {:?}", listener_id)
            }
            _ => println!("unhandled swarm event"),
        }
    }

    pub fn handle_incoming_message(&mut self, message_variant: Message) {
        match message_variant {
            Message::KeepAliveMessage => (),
            Message::ConsensusMessage(consensus_message) => {
                // println!("\nReceived message: {:?}", consensus_message);
                match &self.manager {
                    Some(manager) => {
                        manager.on_artifact(UnvalidatedArtifact::new(
                            consensus_message,
                            system_time_now(),
                        ));
                    }
                    None => (),
                };
            }
        }
    }

    pub fn artifact_manager_started(&self) -> bool {
        self.artifact_manager_started
    }
}
