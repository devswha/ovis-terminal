mod crash;
mod fixture;
mod lifecycle;
mod recovery;
mod refusal;

fn host() -> String {
    ovis_terminal_protocol::domain::update::BuildTarget::host().to_string()
}
