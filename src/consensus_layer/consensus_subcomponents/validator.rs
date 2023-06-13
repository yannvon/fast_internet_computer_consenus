use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use crate::{
    consensus_layer::{
        artifacts::{ChangeAction, ChangeSet, ConsensusMessage, IntoInner},
        consensus::RoundRobin,
        height_index::Height,
        pool_reader::PoolReader,
    },
    time_source::TimeSource,
    FinalizationType, HeightMetrics,
};

pub struct Validator {
    my_node_id: u8,
    _schedule: RoundRobin,
    _time_source: Arc<dyn TimeSource>,
}

impl Validator {
    pub fn new(my_node_id: u8, time_source: Arc<dyn TimeSource>) -> Self {
        Self {
            my_node_id,
            _schedule: RoundRobin::default(),
            _time_source: time_source,
        }
    }

    pub fn on_state_change(
        &self,
        pool_reader: &PoolReader<'_>,
        finalization_times: Arc<RwLock<BTreeMap<Height, Option<HeightMetrics>>>>,
    ) -> (ChangeSet, bool) {
        // println!("\n########## Validator ##########");
        let mut change_set = Vec::new();
        for unvalidated_artifact in pool_reader.pool().unvalidated().artifacts.values() {
            // println!("Validating artifact {:?}", unvalidated_artifact);
            let consensus_message = unvalidated_artifact.to_owned().into_inner();
            if let ConsensusMessage::Finalization(finalization) = &consensus_message {
                // only insert finalization of type DK if received by peer before it was finalized locally
                if !finalization_times
                    .read()
                    .unwrap()
                    .contains_key(&finalization.content.height)
                {
                    if let Some(finalization_time) = pool_reader
                        .get_finalization_time(finalization.content.height, self.my_node_id)
                    {
                        let height_metrics = HeightMetrics {
                            latency: finalization_time,
                            fp_finalization: FinalizationType::DK,
                        };
                        let last_height = *match finalization_times.read().unwrap().last_key_value()
                        {
                            Some((key, _)) => key,
                            None => &0,
                        };
                        if finalization.content.height > last_height || last_height == 0 {
                            finalization_times
                                .write()
                                .unwrap()
                                .insert(finalization.content.height, Some(height_metrics));
                        }
                    }
                }
            }
            change_set.push(ChangeAction::MoveToValidated(consensus_message));
        }
        // the changes due to the validation of a block do not have to be broadcasted as each node performs them locally depending on the state of its consensus pool
        (change_set, false)
    }
}
