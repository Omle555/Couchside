//! Running Couchside as a Windows service.

use anyhow::Context;
use couchside_core::config::{default_config_path, default_data_dir, migrate_legacy_data_dir};
use couchside_core::Config;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

const SERVICE_NAME: &str = "Couchside";
const DISPLAY_NAME: &str = "Couchside Media Server";
const DESCRIPTION: &str = "Streams your movies and TV shows to browsers on your network.";
/// Name of the inbound Windows Firewall rule added for the configured port.
const FIREWALL_RULE: &str = "Couchside Media Server";

define_windows_service!(ffi_service_main, service_main);

/// Entry point used by the Service Control Manager (`couchside.exe service`).
pub fn run_dispatcher() -> anyhow::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("not started by the Windows Service Control Manager; use `couchside run` instead")
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(err) = run_service() {
        tracing::error!(error = %format!("{err:#}"), "service stopped with an error");
    }
}

/// `--config <path>` from the service's launch arguments, if present.
fn config_from_args() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}

fn status(state: ServiceState, exit_code: ServiceExitCode) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        },
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    }
}

fn run_service() -> anyhow::Result<()> {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let mut stop_tx = Some(stop_tx);
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Some(tx) = stop_tx.take() {
                let _ = tx.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, handler)?;
    status_handle.set_service_status(status(ServiceState::Running, ServiceExitCode::Win32(0)))?;

    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the async runtime")
        .and_then(|runtime| {
            runtime.block_on(crate::app::run(
                config_from_args(),
                async {
                    let _ = stop_rx.await;
                },
                false,
                false,
            ))
        });

    let exit = if result.is_ok() {
        ServiceExitCode::Win32(0)
    } else {
        ServiceExitCode::ServiceSpecific(1)
    };
    status_handle.set_service_status(status(ServiceState::Stopped, exit))?;
    result
}

/// Registers Couchside as an automatic-start service and starts it.
pub fn install(config: Option<PathBuf>) -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context("could not open the service manager (run this from an Administrator terminal)")?;

    // Before loading the config, so it doesn't create an empty new data folder.
    if let Some(old) =
        migrate_legacy_data_dir().context("could not move the old Solidfin data folder")?
    {
        println!(
            "Moved {} to {}",
            old.display(),
            default_data_dir().display()
        );
    }
    let port = Config::load_or_create(config.as_deref().unwrap_or(&default_config_path()))?
        .server
        .port;
    let mut launch_arguments = vec![OsString::from("service")];
    if let Some(path) = config {
        let absolute = std::path::absolute(&path).unwrap_or(path);
        launch_arguments.push(OsString::from("--config"));
        launch_arguments.push(absolute.into_os_string());
    }
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: std::env::current_exe()?,
        launch_arguments,
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };
    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)
        .context("could not create the service (is it already installed?)")?;
    service.set_description(DESCRIPTION)?;
    service.start::<&OsStr>(&[])?;
    println!("Couchside service installed and started. It will start automatically with Windows.");
    allow_through_firewall(port);
    Ok(())
}

/// Stops and removes the service.
pub fn uninstall() -> anyhow::Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("could not open the service manager (run this from an Administrator terminal)")?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
        )
        .context("the Couchside service is not installed")?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        service.stop()?;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(500));
            if service.query_status()?.current_state == ServiceState::Stopped {
                break;
            }
        }
    }
    service.delete()?;
    remove_firewall_rule();
    println!("Couchside service removed. Your libraries and settings were kept.");
    Ok(())
}

/// Adds an inbound rule for `port` on private networks, limited to the local subnet (best effort).
fn allow_through_firewall(port: u16) {
    remove_firewall_rule();
    let added = netsh(&[
        "add",
        "rule",
        &format!("name={FIREWALL_RULE}"),
        "dir=in",
        "action=allow",
        "protocol=TCP",
        &format!("localport={port}"),
        "profile=private",
        "remoteip=localsubnet",
    ]);
    if added {
        println!("Windows Firewall now allows devices on your home network to reach port {port}.");
    } else {
        println!("Could not add a Windows Firewall rule for port {port}; see README.md to add it by hand.");
    }
}

/// Removes the firewall rule added by `install-service`, if present.
fn remove_firewall_rule() {
    netsh(&["delete", "rule", &format!("name={FIREWALL_RULE}")]);
}

/// Runs `netsh advfirewall firewall <args>`; true on success. Arguments are passed as a list.
fn netsh(args: &[&str]) -> bool {
    match std::process::Command::new("netsh")
        .args(["advfirewall", "firewall"])
        .args(args)
        .output()
    {
        Ok(output) => output.status.success(),
        Err(err) => {
            tracing::debug!(error = %err, "could not run netsh");
            false
        }
    }
}
