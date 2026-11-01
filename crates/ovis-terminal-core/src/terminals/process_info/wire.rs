//! The published form of an observation: `ProcessRoles` with explicit unsupported values.

use ovis_terminal_protocol::domain::process_info::{
    Observed, ProcessRoles, ProcessSample as WireSample, UnsupportedReason,
};
use ovis_terminal_protocol::schema::ids::{ProcessPid, StartToken};

use crate::terminals::{Field, ProcessInfo, ProcessSample, Unavailable};

const fn reason(unavailable: Unavailable) -> UnsupportedReason {
    match unavailable {
        Unavailable::Platform => UnsupportedReason::Platform,
        Unavailable::UndeclaredRole => UnsupportedReason::UndeclaredRole,
        Unavailable::ProcessUnavailable => UnsupportedReason::ProcessUnavailable,
        Unavailable::Raced => UnsupportedReason::Raced,
        Unavailable::Permission => UnsupportedReason::Permission,
        Unavailable::Encoding => UnsupportedReason::Encoding,
        Unavailable::Limit => UnsupportedReason::Limit,
    }
}

fn observed<T, U>(
    field: &Field<T>,
    convert: impl FnOnce(&T) -> Result<U, Unavailable>,
) -> Observed<U> {
    match field {
        Field::Known(value) => {
            convert(value).map_or_else(|why| Observed::unsupported(reason(why)), Observed::known)
        }
        Field::Unsupported(why) => Observed::unsupported(reason(*why)),
    }
}

fn wire(sample: &ProcessSample) -> Result<WireSample, Unavailable> {
    let pid = u32::try_from(sample.process.pid().get()).map_err(|_| Unavailable::Encoding)?;
    let mut wire = WireSample::new(
        ProcessPid::new(pid),
        StartToken::new(sample.process.start_token().as_str().to_owned()),
        UnsupportedReason::Platform,
    );
    wire.argv = observed(&sample.argv, |argv| Ok(argv.clone()));
    wire.cwd = observed(&sample.cwd, |cwd| {
        cwd.to_str().map(str::to_owned).ok_or(Unavailable::Encoding)
    });
    Ok(wire)
}

fn wire_all(samples: &[ProcessSample]) -> Result<Vec<WireSample>, Unavailable> {
    samples.iter().map(wire).collect()
}

impl ProcessInfo {
    /// The roles as published by `terminal.process_info`.
    #[must_use]
    pub fn roles(&self) -> ProcessRoles {
        let mut roles = ProcessRoles::unsupported(UnsupportedReason::UndeclaredRole);
        roles.shell = observed(&self.shell, wire);
        roles.launcher = observed(&self.launcher, wire);
        roles.foreground = observed(&self.foreground, |samples| wire_all(samples));
        roles.background = observed(&self.background, |samples| wire_all(samples));
        roles
    }
}
