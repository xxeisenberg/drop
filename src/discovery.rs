use local_ip_address::local_ip;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;
use anyhow::{Result, Context};

pub fn spawn_mdns_advertiser(port: u16, mode: &'static str, token: CancellationToken) {
    tokio::spawn(async move {
        let result: Result<()> = async {
            let mdns = ServiceDaemon::new()
                .context("Failed to create mDNS service daemon")?;

            let service_type = "_dropshare._tcp.local.";
            let instance_name = format!("DropShare-{}", mode);
            let host_name = format!("{}.local.", instance_name);

            let mut properties = HashMap::new();
            properties.insert("mode".to_string(), mode.to_string());

            let my_ip = local_ip()
                .context("Failed to determine local IP address for broadcasting")?
                .to_string();

            let service_info = ServiceInfo::new(
                service_type,
                &instance_name,
                &host_name,
                my_ip,
                port,
                Some(properties),
            )
            .context("Failed to construct mDNS service info")?;

            mdns.register(service_info)
                .context("Failed to register mDNS service on the local network")?;

            println!(
                "[ mDNS ] : Broadcasting as '{}' on the local network",
                instance_name
            );

            token.cancelled().await;

            let full_name = format!("{}.{}", instance_name, service_type);
            mdns.unregister(&full_name)
                .context("Failed to gracefully unregister mDNS service")?;

            Ok(())
        }
        .await;

        if let Err(e) = result {
            eprintln!("[ ERROR ] : mDNS advertiser stopped unexpectedly:\n{:#}", e);
        }
    });
}
