//! Minimal host identity collector — the foundation's end-to-end demonstration
//! of the transport → collector → model path. Expanded into the full system
//! collector in v0.1 (phase 1).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use systui_core::{Collector, CommandSpec, ModuleId, Result, Transport};

/// Basic identity of a host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    pub kernel: String,
}

/// Collects [`HostInfo`] via `uname`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HostInfoCollector;

#[async_trait]
impl Collector for HostInfoCollector {
    type Output = HostInfo;

    fn module(&self) -> ModuleId {
        ModuleId::System
    }

    async fn collect(&self, transport: &dyn Transport) -> Result<HostInfo> {
        let hostname = run_trimmed(transport, "uname", &["-n"]).await?;
        let kernel = run_trimmed(transport, "uname", &["-r"]).await?;
        Ok(HostInfo { hostname, kernel })
    }
}

/// Run a command and return its trimmed stdout, erroring on non-zero exit.
async fn run_trimmed(transport: &dyn Transport, program: &str, args: &[&str]) -> Result<String> {
    let spec = CommandSpec::new(program).args(args.iter().copied());
    let output = transport.run(&spec).await?.into_result(program)?;
    Ok(output.stdout.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use systui_core::CoreError;
    use systui_transport::MockTransport;

    #[tokio::test]
    async fn collects_hostname_and_kernel() {
        let transport = MockTransport::new()
            .with_stdout("uname -n", "prod-01\n")
            .with_stdout("uname -r", "6.1.0-18-amd64\n");

        let info = HostInfoCollector.collect(&transport).await.unwrap();
        assert_eq!(info.hostname, "prod-01");
        assert_eq!(info.kernel, "6.1.0-18-amd64");
    }

    #[tokio::test]
    async fn unconfigured_command_propagates_error() {
        let transport = MockTransport::new();
        let err = HostInfoCollector.collect(&transport).await.unwrap_err();
        assert!(matches!(err, CoreError::Transport(_)));
    }
}
