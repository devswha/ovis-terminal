//! Versioned prompts and independently authorized answers (plan todo 34).
//!
//! The owning program opens, republishes and closes its prompts with its process token.
//! Any principal the dispatcher authorized for `prompt_answer` may answer one version once;
//! holding a terminal address, interact permission or a blocked status grants nothing here.

mod answer;
mod error;
mod input;
mod publish;
mod request;
mod service;
mod store;

pub use error::PromptError;
pub use input::{AnswerKeys, KeyInput, KeyInputError};
pub use request::{Answer, Close, Open};
pub use service::PromptService;

#[cfg(test)]
mod qa;
