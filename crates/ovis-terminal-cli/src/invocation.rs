//! One invocation: parse, plan, execute, report.

mod args;
mod compat;
mod context;
mod discovery;
mod error;
mod execute;
mod exit;
mod plan;
mod report;
mod text;
mod transport;

use std::ffi::OsString;

use clap::Parser as _;
use clap::error::ErrorKind;

use self::args::{Cli, SchemaPart};
use self::context::OutputMode;
pub use self::context::{CREDENTIAL_ENV, Context, Credential, JSON_ENV, PROCESS_TOKEN_ENV};
use self::error::CliError;
pub use self::exit::ExitStatus;
use self::plan::Plan;
use self::report::Failure;
pub use self::report::Report;

fn parse(args: Vec<OsString>, mode: OutputMode) -> Result<Cli, Report> {
    Cli::try_parse_from(std::iter::once(OsString::from(crate::BINARY_NAME)).chain(args)).map_err(
        |source| match source.kind() {
            ErrorKind::DisplayVersion => discovery::version(mode),
            ErrorKind::DisplayHelp => match mode {
                OutputMode::Text => Report::raw(source.to_string()),
                OutputMode::Json => {
                    Report::success(mode, &serde_json::json!({ "help": source.to_string() }))
                }
            },
            _ => {
                let rendered = source.to_string();
                let detail = rendered
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim_start_matches("error: ")
                    .to_owned();
                Report::failure(mode, &CliError::Usage { detail, source }.into())
            }
        },
    )
}

async fn execute(plan: Plan, mode: OutputMode, context: &Context) -> Result<Report, Failure> {
    let value = match plan {
        Plan::Skill => return Ok(discovery::skill(mode)),
        Plan::NewOp => return Ok(discovery::new_op(mode)),
        Plan::Schema(SchemaPart::Openrpc) => return Ok(discovery::openrpc()?),
        Plan::Schema(SchemaPart::Cli) => discovery::cli_contract(),
        Plan::Schema(SchemaPart::Contract) => return Ok(discovery::contract(mode)?),
        Plan::Schema(SchemaPart::Compat) => return Err(CliError::ReleasedFlag.into()),
        Plan::Compat(released) => compat::compat(&released)?,
        Plan::Endpoint => discovery::endpoint(context)?,
        Plan::Status => execute::status(context).await?,
        Plan::Call(request) => execute::call(context, &request).await?,
        Plan::NeedsTarget => return Err(execute::needs_target(context).await),
        #[cfg(any(unix, windows))]
        Plan::Service(command) => {
            match crate::service::execute(&command).map_err(CliError::Service)? {
                Some(value) => value,
                None => return Ok(Report::raw(String::new())),
            }
        }
        Plan::Bridge => {
            execute::connect(context).await?;
            return Ok(Report::raw(String::new()));
        }
        #[cfg(unix)]
        Plan::Serve(plan) => {
            crate::assembly::serve(&plan, context.endpoint())
                .await
                .map_err(CliError::Assembly)?;
            return Ok(Report::raw(String::new()));
        }
        #[cfg(unix)]
        Plan::Attach(plan) => return execute::attach(context, &plan).await,
    };
    Ok(Report::success(mode, &value))
}

/// Runs one invocation: `args` excludes the program name.
pub async fn run(args: Vec<OsString>, context: &Context) -> Report {
    let mode = match OutputMode::select(&args, context) {
        Ok(mode) => mode,
        Err(error) => {
            let mode = match &error {
                CliError::JsonSwitch { mode } => *mode,
                _ => OutputMode::Text,
            };
            return Report::failure(mode, &error.into());
        }
    };
    let planned = match parse(args, mode) {
        Ok(cli) => plan::plan(cli, context.process_token()),
        Err(report) => return report,
    };
    let outcome = match planned {
        Ok(plan) => execute(plan, mode, context).await,
        Err(error) => Err(error.into()),
    };
    outcome.unwrap_or_else(|failure| Report::failure(mode, &failure))
}

#[cfg(test)]
mod manual_tests;
