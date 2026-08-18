use std::io;

use super::command::{Command, CommandError};
use super::search_worker::SearchEvent;

pub(super) enum Event {
    Input(Result<Command, CommandError>),
    InputClosed,
    InputFailed(io::Error),
    InputPanicked,
    Search(SearchEvent),
}
