//! Production CLI sessions opened against the production pipe listener: two
//! invocations at once both get served, and an observer invocation is refused every
//! mutation while an owner invocation is served.

use std::sync::atomic::Ordering;

use ovis_terminal_protocol::rpc::{RpcRequest, parse_request};
use ovis_terminal_server::dispatch::lanes::boundary_policy::MUTATION_ROLES;
use serde_json::json;

use super::TestResult;
use super::served::served;
use crate::invocation::report::Failure;
use crate::invocation::transport::Session;

fn request(method: &str) -> TestResult<RpcRequest> {
    let text = json!({"jsonrpc": "2.0", "id": method, "method": method, "params": {}});
    Ok(parse_request(&text.to_string())?)
}

#[tokio::test]
async fn two_invocations_opened_at_once_both_get_a_session_from_the_production_listener()
-> TestResult {
    let served = served()?;
    let (first, second) = tokio::join!(
        Session::open(&served.address, Some(&served.owner)),
        Session::open(&served.address, Some(&served.owner))
    );
    let mut first = first.map_err(|failure| format!("first invocation: {failure:?}"))?;
    let mut second = second.map_err(|failure| format!("second invocation: {failure:?}"))?;
    let get = request("doc.get")?;
    let (a, b) = tokio::join!(first.call(&get), second.call(&get));
    let a = a.map_err(|failure| format!("first call: {failure:?}"))?;
    let b = b.map_err(|failure| format!("second call: {failure:?}"))?;
    assert_eq!(a, json!({"served": "doc.get"}));
    assert_eq!(b, json!({"served": "doc.get"}));
    assert_eq!(served.reached.load(Ordering::SeqCst), 2);
    drop((first, second));
    let report = served.server.shutdown().await?;
    assert_eq!((report.accepted(), report.failures().len()), (2, 0));
    Ok(())
}

#[tokio::test]
async fn an_observer_invocation_is_refused_every_mutation_while_an_owner_is_served() -> TestResult {
    let served = served()?;
    let (observer, owner) = tokio::join!(
        Session::open(&served.address, Some(&served.observer)),
        Session::open(&served.address, Some(&served.owner))
    );
    let mut observer = observer.map_err(|failure| format!("observer: {failure:?}"))?;
    let mut owner = owner.map_err(|failure| format!("owner: {failure:?}"))?;
    let mut refused = 0_usize;
    for (method, _) in MUTATION_ROLES
        .iter()
        .filter(|(name, _)| *name != "attach.open")
    {
        match observer.call(&request(method)?).await {
            Err(Failure::Server(error)) => {
                assert_eq!(error.data.kind.token(), "forbidden", "{method}: {error:?}");
                refused = refused.saturating_add(1);
            }
            other => return Err(format!("{method}: expected forbidden, got {other:?}").into()),
        }
    }
    assert_eq!(refused, MUTATION_ROLES.len().saturating_sub(1));
    assert_eq!(served.reached.load(Ordering::SeqCst), 0, "no mutation ran");
    let set = owner
        .call(&request("doc.set")?)
        .await
        .map_err(|failure| format!("owner doc.set: {failure:?}"))?;
    assert_eq!(set, json!({"served": "doc.set"}));
    assert_eq!(served.reached.load(Ordering::SeqCst), 1);
    drop((observer, owner));
    served.server.shutdown().await?;
    Ok(())
}
