//! Native endpoint and root-secret discovery from the environment (core §3).
//!
//! A native producer learns where to connect, and how to authenticate, from environment variables
//! such as [`ENDPOINT_CONTROL`] and [`ROOT_SECRET`]. [`NativeDiscovery::from_environment`] reads
//! and validates them, and [`NativeDiscovery::from_lookup`] takes them from any source, which keeps
//! tests off the process environment. The secret never appears in `Debug` output.

use std::{env, fmt, io};
use zeroize::Zeroizing;

use crate::{
    auth::{AuthError, Secret32},
    messages::LaneClass,
    wire::Endpoint,
};

/// Environment variable naming the control endpoint.
pub const ENDPOINT_CONTROL: &str = "VIVID_ENDPOINT_CONTROL";
/// Environment variable naming the interactive-lane endpoint.
pub const ENDPOINT_INTERACTIVE: &str = "VIVID_ENDPOINT_INTERACTIVE";
/// Environment variable naming the realtime-media endpoint.
pub const ENDPOINT_REALTIME: &str = "VIVID_ENDPOINT_REALTIME";
/// Environment variable naming the bulk endpoint.
pub const ENDPOINT_BULK: &str = "VIVID_ENDPOINT_BULK";
/// Environment variable holding the root secret as 64 hexadecimal digits.
pub const ROOT_SECRET: &str = "VIVID_ROOT_SECRET";

/// The endpoints and root secret a native producer reads from its environment (core §3).
///
/// # Examples
///
/// ```
/// use std::collections::HashMap;
/// use vivid_protocol::discovery::{ENDPOINT_CONTROL, NativeDiscovery, ROOT_SECRET};
///
/// let environment = HashMap::from([
///     (ENDPOINT_CONTROL, "tcp:127.0.0.1:7000".to_owned()),
///     (ROOT_SECRET, "ab".repeat(32)),
/// ]);
/// let discovery = NativeDiscovery::from_lookup(|name| environment.get(name).cloned())?;
/// assert_eq!(discovery.control().as_tcp().map(|address| address.port()), Some(7000));
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct NativeDiscovery {
    control: Endpoint,
    interactive: Option<Endpoint>,
    realtime: Option<Endpoint>,
    bulk: Option<Endpoint>,
    root_secret: Secret32,
}

impl fmt::Debug for NativeDiscovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeDiscovery")
            .field("control", &"[REDACTED ENDPOINT]")
            .field(
                "interactive",
                &self.interactive.as_ref().map(|_| "[REDACTED ENDPOINT]"),
            )
            .field(
                "realtime",
                &self.realtime.as_ref().map(|_| "[REDACTED ENDPOINT]"),
            )
            .field("bulk", &self.bulk.as_ref().map(|_| "[REDACTED ENDPOINT]"))
            .field("root_secret", &"[REDACTED]")
            .finish()
    }
}

impl NativeDiscovery {
    /// Reads discovery from the process environment.
    ///
    /// A variable that is absent or not valid Unicode counts as unset.
    ///
    /// # Errors
    ///
    /// Same as [`Self::from_lookup`].
    pub fn from_environment() -> io::Result<Self> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    /// Reads discovery through `lookup`, which maps a variable name to its value.
    ///
    /// This is the environment-free form of [`Self::from_environment`], for embedders that
    /// receive discovery some other way and for tests.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` when the control endpoint or root secret is unset, `InvalidInput` for a
    /// malformed endpoint, and `InvalidData` for a root secret that is not 64 hex digits.
    pub fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> io::Result<Self> {
        let control = required_endpoint(&mut lookup, ENDPOINT_CONTROL)?;
        let interactive = optional_endpoint(&mut lookup, ENDPOINT_INTERACTIVE)?;
        let realtime = optional_endpoint(&mut lookup, ENDPOINT_REALTIME)?;
        let bulk = optional_endpoint(&mut lookup, ENDPOINT_BULK)?;
        let value = Zeroizing::new(lookup(ROOT_SECRET).ok_or_else(|| missing(ROOT_SECRET))?);
        let root_secret =
            Secret32::from_hex(&value).map_err(|error| secret_error(ROOT_SECRET, error))?;
        Ok(Self {
            control,
            interactive,
            realtime,
            bulk,
            root_secret,
        })
    }

    /// Returns the control endpoint.
    pub fn control(&self) -> &Endpoint {
        &self.control
    }

    /// Returns the interactive-lane endpoint.
    pub fn interactive(&self) -> &Endpoint {
        self.interactive.as_ref().unwrap_or(&self.control)
    }

    /// Returns the endpoint a track on `lane` connects to.
    ///
    /// It falls back to the bulk endpoint and then to the control endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] when `lane` is neither realtime nor bulk.
    pub fn track(&self, lane: LaneClass) -> io::Result<&Endpoint> {
        match lane {
            LaneClass::Realtime => Ok(self
                .realtime
                .as_ref()
                .or(self.bulk.as_ref())
                .unwrap_or(&self.control)),
            LaneClass::Bulk => Ok(self.bulk.as_ref().unwrap_or(&self.control)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "track endpoint requires realtime or bulk lane",
            )),
        }
    }

    /// Returns the root secret; keep it out of logs and arguments.
    pub fn root_secret(&self) -> &Secret32 {
        &self.root_secret
    }
}

fn required_endpoint(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
) -> io::Result<Endpoint> {
    lookup(name)
        .ok_or_else(|| missing(name))
        .and_then(|value| Endpoint::parse(&value).map_err(io::Error::from))
}

fn optional_endpoint(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
) -> io::Result<Option<Endpoint>> {
    lookup(name)
        .map(|value| Endpoint::parse(&value).map_err(io::Error::from))
        .transpose()
}

fn missing(name: &'static str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("required Vivid discovery variable {name} is absent"),
    )
}

fn secret_error(name: &'static str, error: AuthError) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{name} has invalid encoding: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_exposes_discovery_values() {
        let discovery = NativeDiscovery {
            control: Endpoint::parse("tcp:127.0.0.1:1234").unwrap(),
            interactive: None,
            realtime: None,
            bulk: None,
            root_secret: Secret32::new([7; 32]),
        };
        let debug = format!("{discovery:?}");
        assert!(!debug.contains("1234"));
        assert!(!debug.contains("0707"));
    }

    fn lookup<'a>(vars: &'a [(&str, &str)]) -> impl FnMut(&str) -> Option<String> + 'a {
        |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn lookup_reads_discovery_without_the_process_environment() {
        let secret = "0123456789abcdef".repeat(4);
        let discovery = NativeDiscovery::from_lookup(lookup(&[
            (ENDPOINT_CONTROL, "tcp:127.0.0.1:1234"),
            (ENDPOINT_BULK, "tcp:127.0.0.1:5678"),
            (ROOT_SECRET, &secret),
        ]))
        .unwrap();
        assert_eq!(
            discovery.control(),
            &Endpoint::parse("tcp:127.0.0.1:1234").unwrap()
        );
        assert_eq!(discovery.interactive(), discovery.control());
        assert_eq!(
            discovery.track(LaneClass::Bulk).unwrap(),
            &Endpoint::parse("tcp:127.0.0.1:5678").unwrap()
        );
        assert_eq!(
            discovery.root_secret().expose(),
            Secret32::from_hex(&secret).unwrap().expose()
        );
    }

    #[test]
    fn lookup_rejects_missing_or_malformed_values_without_echoing_secrets() {
        let missing_control = NativeDiscovery::from_lookup(lookup(&[(ROOT_SECRET, "00")]));
        assert_eq!(missing_control.unwrap_err().kind(), io::ErrorKind::NotFound);

        let missing_secret =
            NativeDiscovery::from_lookup(lookup(&[(ENDPOINT_CONTROL, "tcp:127.0.0.1:1234")]));
        assert_eq!(missing_secret.unwrap_err().kind(), io::ErrorKind::NotFound);

        let bad_secret = "zz".repeat(32);
        let error = NativeDiscovery::from_lookup(lookup(&[
            (ENDPOINT_CONTROL, "tcp:127.0.0.1:1234"),
            (ROOT_SECRET, &bad_secret),
        ]))
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!error.to_string().contains(&bad_secret));
    }

    #[test]
    fn fallback_selects_values_without_inventing_endpoint_retries() {
        let discovery = NativeDiscovery {
            control: Endpoint::parse("tcp:127.0.0.1:1234").unwrap(),
            interactive: None,
            realtime: None,
            bulk: Some(Endpoint::parse("tcp:127.0.0.1:5678").unwrap()),
            root_secret: Secret32::new([7; 32]),
        };
        assert_eq!(discovery.interactive(), discovery.control());
        assert_eq!(
            discovery.track(LaneClass::Realtime).unwrap(),
            discovery.track(LaneClass::Bulk).unwrap()
        );
    }
}
