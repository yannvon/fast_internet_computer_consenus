use crate::artifact_manager::ProcessingResult;
use crate::{HeightMetrics, SubnetParams};

pub mod pool;
use crate::consensus_layer::pool::ConsensusPoolImpl;

pub mod consensus;
use crate::consensus_layer::consensus::ConsensusImpl;

pub mod artifacts;
use crate::consensus_layer::artifacts::{ChangeAction, ConsensusMessage, UnvalidatedArtifact};

pub mod pool_reader;

pub mod height_index;

pub mod consensus_subcomponents;

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use self::height_index::Height;

pub struct ConsensusProcessor {
    consensus_pool: Arc<RwLock<ConsensusPoolImpl>>,
    client: Box<ConsensusImpl>,
}

impl ConsensusProcessor {
    pub fn new(replica_number: u8, subnet_params: SubnetParams) -> Self {
        Self {
            consensus_pool: Arc::new(RwLock::new(ConsensusPoolImpl::new())),
            client: Box::new(ConsensusImpl::new(replica_number, subnet_params)),
        }
    }

    pub fn process_changes(
        &self,
        artifacts: Vec<UnvalidatedArtifact<ConsensusMessage>>,
        finalization_times: Arc<RwLock<BTreeMap<Height, Option<HeightMetrics>>>>,
    ) -> (Vec<ConsensusMessage>, ProcessingResult) {
        {
            let mut consensus_pool = self.consensus_pool.write().unwrap();
            for artifact in artifacts {
                consensus_pool.insert(artifact)
            }
        }
        let mut adverts = Vec::new();
        let (change_set, to_broadcast) = {
            let consensus_pool = self.consensus_pool.read().unwrap();
            self.client
                .on_state_change(&consensus_pool, finalization_times)
        };
        let changed = if !change_set.is_empty() {
            ProcessingResult::StateChanged
        } else {
            ProcessingResult::StateUnchanged
        };

        if to_broadcast {
            for change_action in change_set.iter() {
                match change_action {
                    ChangeAction::AddToValidated(to_add) => {
                        // println!("Broadcasting consensus message to be added: {:?}", to_add);
                        adverts.push(to_add.to_owned());
                    }
                    ChangeAction::MoveToValidated(to_move) => {
                        // println!("Broadcasting consensus message to be moved: {:?}", to_move);
                        adverts.push(to_move.to_owned());
                    }
                }
            }
        }

        // if !change_set.is_empty() {
        //     println!("\n########## Processor ##########");
        //     println!("Applying change set: {:?}", change_set);
        // }

        self.consensus_pool
            .write()
            .unwrap()
            .apply_changes(change_set);

        (adverts, changed)
    }
}
