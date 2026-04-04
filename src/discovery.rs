use anyhow::{Context, Result};
use local_ip_address::local_ip;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

pub fn get_mdns_names(mode: &str) -> (String, String) {
    let pc_name = whoami::devicename()
        .unwrap_or_else(|_| whoami::hostname().unwrap_or_else(|_| "Host".to_string()));
    let suffix = format!(" (DropShare-{})", mode);
    let max_instance_len: usize = 63;

    let mut instance_name = String::new();
    let max_prefix_len = max_instance_len.saturating_sub(suffix.len());
    for c in pc_name.chars() {
        if instance_name.len() + c.len_utf8() > max_prefix_len {
            break;
        }
        instance_name.push(c);
    }

    if instance_name.is_empty() && suffix.len() > max_instance_len {
        for c in suffix.chars() {
            if instance_name.len() + c.len_utf8() > max_instance_len {
                break;
            }
            instance_name.push(c);
        }
    } else {
        instance_name.push_str(&suffix);
    }

    let host_name = format!("{}.local.", instance_name.replace(' ', "-"));
    (instance_name, host_name)
}

pub fn spawn_mdns_advertiser(
    port: u16,
    mode: &'static str,
    auth_token: Option<String>,
    enc_key: Option<String>,
    token: CancellationToken,
) {
    tokio::spawn(async move {
        let result: Result<()> = async {
            let mdns = ServiceDaemon::new().context("Failed to create mDNS service daemon")?;

            let service_type = "_dropshare._tcp.local.";
            let (instance_name, host_name) = get_mdns_names(mode);

            let mut properties = HashMap::new();
            properties.insert("mode".to_string(), mode.to_string());
            if let Some(ref auth_token) = auth_token {
                properties.insert("token".to_string(), auth_token.clone());
            }
            if let Some(ref key) = enc_key {
                properties.insert("enc_key".to_string(), key.clone());
            }

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
