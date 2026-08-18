use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::engine::{Engine, SearchControl, SearchInfo, SearchLimits, SearchResult};

use super::event::Event;

#[derive(Debug)]
pub(super) enum SearchEvent {
    Info {
        generation: u64,
        info: SearchInfo,
    },
    Finished {
        generation: u64,
        result: SearchResult,
    },
    Failed {
        generation: u64,
    },
}

pub(super) struct SearchTask {
    generation: u64,
    control: SearchControl,
    handle: Option<JoinHandle<()>>,
    pending: Option<SearchResult>,
    release_on_finish: bool,
    pondering: bool,
    ponder_budget: Option<Duration>,
    stop_on_ponder_hit: bool,
}

impl SearchTask {
    pub(super) fn spawn(
        engine: Engine,
        limits: SearchLimits,
        generation: u64,
        sender: Sender<Event>,
    ) -> Self {
        let control = SearchControl::new();
        let worker_control = control.clone();
        let report_control = control.clone();
        let info_sender = sender.clone();
        let pondering = limits.ponder;
        let release_on_finish = !limits.infinite && !pondering;
        let ponder_budget = if pondering {
            engine.ponder_time_budget(&limits)
        } else {
            None
        };
        let stop_on_ponder_hit = pondering
            && ponder_budget.is_none()
            && limits.depth.is_none()
            && limits.nodes.is_none()
            && limits.mate.is_none()
            && !limits.infinite;
        let handle = thread::spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| {
                engine.search_with_reporter(&limits, &worker_control, |info| {
                    if info_sender
                        .send(Event::Search(SearchEvent::Info { generation, info }))
                        .is_err()
                    {
                        report_control.stop();
                    }
                })
            }));
            let event = match result {
                Ok(result) => SearchEvent::Finished { generation, result },
                Err(_) => SearchEvent::Failed { generation },
            };
            let _ = sender.send(Event::Search(event));
        });

        Self {
            generation,
            control,
            handle: Some(handle),
            pending: None,
            release_on_finish,
            pondering,
            ponder_budget,
            stop_on_ponder_hit,
        }
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn complete(&mut self, result: SearchResult) -> Option<SearchResult> {
        self.join();
        if self.release_on_finish {
            Some(result)
        } else {
            self.pending = Some(result);
            None
        }
    }

    pub(super) fn fail(&mut self) -> Option<SearchResult> {
        self.join();
        let result = SearchResult::default();
        if self.release_on_finish {
            Some(result)
        } else {
            self.pending = Some(result);
            None
        }
    }

    pub(super) fn stop_and_release(&mut self) -> Option<SearchResult> {
        self.release_on_finish = true;
        self.control.stop();
        self.pending.take()
    }

    pub(super) fn ponder_hit(&mut self) -> Option<SearchResult> {
        if !self.pondering {
            return None;
        }

        self.pondering = false;
        self.release_on_finish = true;
        if let Some(result) = self.pending.take() {
            return Some(result);
        }
        if let Some(duration) = self.ponder_budget {
            self.control.set_deadline_from_now(duration);
        } else if self.stop_on_ponder_hit {
            self.control.stop();
        }
        None
    }

    pub(super) fn cancel(&mut self) {
        self.control.stop();
        self.join();
        self.pending = None;
    }

    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
