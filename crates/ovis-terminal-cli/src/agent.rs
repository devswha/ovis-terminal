//! Typed verbs of the agent domain: agents, status, prompts, turns and templates.
//!
//! Each one is a published method with a closed params type, and its flags are exactly the
//! ones that type takes (`--op-id` for a mutation, a target for a terminal-scoped method).
//! `--params` carries the rest.

use clap::Subcommand;

use crate::commands::{Mutate, MutatePlain, Plain, Read, Supplied};

pub mod table;

pub(crate) use table::VERBS;

/// `ovis-terminal (agent | status | prompt | turn | template) <verb>`.
#[derive(Debug, Subcommand)]
pub(crate) enum DomainCommand {
    /// Declared agents: registration, session binding, children, notes and records.
    #[command(subcommand)]
    Agent(AgentVerb),
    /// Status reported by the program that owns a terminal.
    #[command(subcommand)]
    Status(StatusVerb),
    /// Versioned prompts a program opened and their answers.
    #[command(subcommand)]
    Prompt(PromptVerb),
    /// Turn records of a terminal's agent.
    #[command(subcommand)]
    Turn(TurnVerb),
    /// Launch templates by stable name and version.
    #[command(subcommand)]
    Template(TemplateVerb),
}

/// `ovis-terminal agent ...`.
#[derive(Debug, Subcommand)]
pub(crate) enum AgentVerb {
    /// Register the program that owns the terminal as an agent (its process token).
    Register(Mutate),
    /// Read the agent of one terminal; `agent` is null for a plain program.
    Get(Read),
    /// List registered agents.
    List(Plain),
    /// Bind or replace the agent's provider session reference.
    UpdateSession(Mutate),
    /// End the agent registration; the terminal keeps running.
    Release(Mutate),
    /// List the agent's child terminals and their lifecycle facts.
    Children(Read),
    /// Deliver a note from the agent to one of its children.
    Note(Mutate),
    /// Wait until the agent's children settle, within a bound.
    WaitChildren(Read),
    /// Read the agent's conversation blocks from a cursor.
    Conversation(Read),
    /// Append conversation records reported by the owning program.
    AppendRecords(Mutate),
    /// Read the agent's appended records from a cursor.
    Records(Read),
    /// List provider sessions in the authorized store as inert data.
    History(Plain),
}

/// `ovis-terminal status ...`.
#[derive(Debug, Subcommand)]
pub(crate) enum StatusVerb {
    /// Read the declared status of one terminal.
    Get(Read),
    /// Report status as the owning program; a stale `seq` is refused.
    Report(Mutate),
    /// Clear the reported status as the owning program.
    Clear(Mutate),
}

/// `ovis-terminal prompt ...`.
#[derive(Debug, Subcommand)]
pub(crate) enum PromptVerb {
    /// List prompts, optionally of one terminal.
    List(Read),
    /// Read one prompt by identity.
    Get(Plain),
    /// Open a typed prompt as the owning program.
    Open(Mutate),
    /// Answer a prompt at its current version; needs the prompt-answer grant.
    Answer(MutatePlain),
    /// Close a prompt as the owning program.
    Close(MutatePlain),
}

/// `ovis-terminal turn ...`.
#[derive(Debug, Subcommand)]
pub(crate) enum TurnVerb {
    /// Read the latest turn record of one terminal.
    Get(Read),
    /// Wait for a turn to end, within a bound.
    Wait(Read),
}

/// `ovis-terminal template ...`.
#[derive(Debug, Subcommand)]
pub(crate) enum TemplateVerb {
    /// List launch templates.
    List(Plain),
    /// Read one template by name and optional version.
    Get(Plain),
}

impl DomainCommand {
    /// The method this verb sends and the flags it was given.
    pub(crate) fn resolve(self) -> (&'static str, Supplied) {
        match self {
            Self::Agent(verb) => verb.resolve(),
            Self::Status(verb) => verb.resolve(),
            Self::Prompt(verb) => verb.resolve(),
            Self::Turn(TurnVerb::Get(args)) => ("turn.get", args.into()),
            Self::Turn(TurnVerb::Wait(args)) => ("turn.wait", args.into()),
            Self::Template(TemplateVerb::List(args)) => ("template.list", args.into()),
            Self::Template(TemplateVerb::Get(args)) => ("template.get", args.into()),
        }
    }
}

impl AgentVerb {
    fn resolve(self) -> (&'static str, Supplied) {
        match self {
            Self::Register(args) => ("agent.register", args.into()),
            Self::Get(args) => ("agent.get", args.into()),
            Self::List(args) => ("agent.list", args.into()),
            Self::UpdateSession(args) => ("agent.update_session", args.into()),
            Self::Release(args) => ("agent.release", args.into()),
            Self::Children(args) => ("agent.children", args.into()),
            Self::Note(args) => ("agent.note", args.into()),
            Self::WaitChildren(args) => ("agent.wait_children", args.into()),
            Self::Conversation(args) => ("agent.conversation", args.into()),
            Self::AppendRecords(args) => ("agent.append_records", args.into()),
            Self::Records(args) => ("agent.records", args.into()),
            Self::History(args) => ("agent.history", args.into()),
        }
    }
}

impl StatusVerb {
    fn resolve(self) -> (&'static str, Supplied) {
        match self {
            Self::Get(args) => ("status.get", args.into()),
            Self::Report(args) => ("status.report", args.into()),
            Self::Clear(args) => ("status.clear", args.into()),
        }
    }
}

impl PromptVerb {
    fn resolve(self) -> (&'static str, Supplied) {
        match self {
            Self::List(args) => ("prompt.list", args.into()),
            Self::Get(args) => ("prompt.get", args.into()),
            Self::Open(args) => ("prompt.open", args.into()),
            Self::Answer(args) => ("prompt.answer", args.into()),
            Self::Close(args) => ("prompt.close", args.into()),
        }
    }
}

#[cfg(test)]
#[path = "agent/tests.rs"]
mod tests;
