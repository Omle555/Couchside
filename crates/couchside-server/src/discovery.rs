//! Announces Couchside on the local network with DNS-SD over mDNS (`_couchside._tcp.local`),
//! so Home Assistant and other apps can find it. On Windows it uses the mDNS responder built into
//! Windows (10 1809 and later); on Linux it hands a service file to Avahi (`avahi-daemon`).

use std::net::{IpAddr, Ipv4Addr};

/// DNS-SD service type Couchside is announced as.
pub const SERVICE_TYPE: &str = "_couchside._tcp.local";

/// What to announce.
#[derive(Debug, Clone)]
pub struct Announcement {
    /// Instance name shown to people, e.g. `Couchside on ljdesktop`.
    pub name: String,
    /// This PC's mDNS host name, e.g. `ljdesktop.local`.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub host: String,
    /// IPv4 address to publish for `host`, if known.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub ip4: Option<Ipv4Addr>,
    /// HTTP port.
    pub port: u16,
    /// TXT record entries.
    pub txt: Vec<(String, String)>,
}

impl Announcement {
    /// The standard announcement for this server.
    pub fn for_server(computer: &str, ip4: Option<Ipv4Addr>, port: u16) -> Self {
        Self {
            name: format!("Couchside on {computer}"),
            host: format!("{computer}.local"),
            ip4,
            port,
            txt: vec![
                ("version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
                ("api".to_string(), "/api/v1".to_string()),
                ("server".to_string(), computer.to_string()),
            ],
        }
    }

    /// Full DNS-SD instance name, e.g. `Couchside on ljdesktop._couchside._tcp.local`.
    pub fn instance_name(&self) -> String {
        format!("{}.{SERVICE_TYPE}", self.name)
    }
}

/// A running announcement; withdrawn from the network by [`Discovery::stop`].
pub struct Discovery {
    stop: std::sync::mpsc::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl Discovery {
    /// Starts announcing. Failures are logged; Couchside works without discovery.
    pub fn start(announcement: Announcement) -> Option<Self> {
        let (stop, stopped) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("couchside-discovery".to_string())
            .spawn(move || platform::run(&announcement, &stopped))
            .map_err(|err| tracing::warn!(error = %err, "could not start network discovery"))
            .ok()?;
        Some(Self { stop, thread })
    }

    /// Withdraws the announcement (a "goodbye" so other devices forget it quickly).
    pub async fn stop(self) {
        let _ = self.stop.send(());
        let thread = self.thread;
        if let Err(err) = tokio::task::spawn_blocking(move || thread.join()).await {
            tracing::debug!(error = %err, "network discovery thread did not finish cleanly");
        }
    }
}

/// Starts announcing a server listening on `bind`, unless only this PC can connect.
pub async fn start_for(bind: IpAddr, port: u16) -> Option<Discovery> {
    if bind.is_loopback() {
        tracing::debug!("network discovery is off: only this PC can connect");
        return None;
    }
    let ip4 = match bind {
        IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
        IpAddr::V6(v6) if !v6.is_unspecified() => None,
        _ => match crate::lan::primary_address().await {
            Ok(IpAddr::V4(v4)) => Some(v4),
            Ok(IpAddr::V6(_)) => None,
            Err(err) => {
                tracing::debug!(error = %err, "could not find this PC's network address");
                None
            }
        },
    };
    let Some(computer) = crate::lan::computer_name() else {
        tracing::warn!("network discovery is off: this PC's name is unknown");
        return None;
    };
    Discovery::start(Announcement::for_server(&computer, ip4, port))
}

#[cfg(windows)]
mod platform {
    use super::Announcement;
    use std::ffi::c_void;
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use std::time::Duration;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::DNS_REQUEST_PENDING;
    use windows::Win32::NetworkManagement::Dns::{
        DnsServiceConstructInstance, DnsServiceDeRegister, DnsServiceFreeInstance,
        DnsServiceRegister, DNS_QUERY_REQUEST_VERSION1, DNS_SERVICE_INSTANCE,
        DNS_SERVICE_REGISTER_REQUEST,
    };

    /// How long to wait for Windows to confirm a registration or withdrawal.
    const REPLY_TIME: Duration = Duration::from_secs(10);

    /// Null-terminated UTF-16 copy of `s`.
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Called by Windows when a registration or withdrawal finishes.
    unsafe extern "system" fn on_complete(
        status: u32,
        context: *const c_void,
        instance: *const DNS_SERVICE_INSTANCE,
    ) {
        if !instance.is_null() {
            // SAFETY: Windows hands us a copy of the instance that the callee must free.
            unsafe { DnsServiceFreeInstance(instance) };
        }
        // SAFETY: `context` is the `SyncSender` boxed in `run`, which stays alive until the
        // final callback has arrived (or is leaked if it never does).
        let replies = unsafe { &*(context as *const SyncSender<u32>) };
        let _ = replies.try_send(status);
    }

    /// Registers the service, waits for `stop`, then withdraws it.
    pub fn run(announcement: &Announcement, stop: &Receiver<()>) {
        let name = wide(&announcement.instance_name());
        let host = wide(&announcement.host);
        let keys: Vec<Vec<u16>> = announcement.txt.iter().map(|(k, _)| wide(k)).collect();
        let values: Vec<Vec<u16>> = announcement.txt.iter().map(|(_, v)| wide(v)).collect();
        let key_ptrs: Vec<PCWSTR> = keys.iter().map(|k| PCWSTR(k.as_ptr())).collect();
        let value_ptrs: Vec<PCWSTR> = values.iter().map(|v| PCWSTR(v.as_ptr())).collect();
        // Windows wants the address in network byte order.
        let ip4 = announcement.ip4.map(|ip| u32::from_ne_bytes(ip.octets()));

        // SAFETY: every pointer refers to a live, null-terminated buffer owned above, and the
        // key and value arrays hold exactly `txt.len()` entries. Windows copies the strings.
        let instance = unsafe {
            DnsServiceConstructInstance(
                PCWSTR(name.as_ptr()),
                PCWSTR(host.as_ptr()),
                ip4.as_ref().map(|ip| ip as *const u32),
                None,
                announcement.port,
                0,
                0,
                key_ptrs.len() as u32,
                key_ptrs.as_ptr(),
                value_ptrs.as_ptr(),
            )
        };
        if instance.is_null() {
            tracing::warn!("could not announce Couchside on the network (bad service details)");
            return;
        }

        let (reply_tx, replies) = sync_channel::<u32>(4);
        let context = Box::new(reply_tx);
        let request = DNS_SERVICE_REGISTER_REQUEST {
            Version: DNS_QUERY_REQUEST_VERSION1.0,
            InterfaceIndex: 0,
            pServiceInstance: instance,
            pRegisterCompletionCallback: Some(on_complete),
            pQueryContext: &*context as *const SyncSender<u32> as *mut c_void,
            ..Default::default()
        };

        // SAFETY: `request`, `instance` and `context` stay alive until Windows has finished
        // with them: they are released below only once no callback can still arrive.
        let status = unsafe { DnsServiceRegister(&request, None) };
        let finished = if status != DNS_REQUEST_PENDING as u32 {
            tracing::warn!(
                code = status,
                "could not announce Couchside on the network (needs Windows 10 1809 or later)"
            );
            true
        } else {
            match replies.recv_timeout(REPLY_TIME) {
                Ok(0) => {
                    tracing::info!(
                        name = %announcement.instance_name(),
                        port = announcement.port,
                        "announced on the local network (Home Assistant can discover it)"
                    );
                    let _ = stop.recv();
                    withdraw(&request, &replies)
                }
                Ok(code) => {
                    tracing::warn!(code, "Windows refused the network announcement");
                    true
                }
                Err(_) => {
                    tracing::warn!("Windows did not confirm the network announcement");
                    false
                }
            }
        };

        if finished {
            // SAFETY: Windows has reported back (or never accepted the request), so it no
            // longer uses the instance it was given.
            unsafe { DnsServiceFreeInstance(instance) };
        } else {
            // A callback may still arrive: keep what it reads alive rather than free it early.
            std::mem::forget(context);
        }
    }

    /// Withdraws a registered service. Returns false if Windows never confirmed it.
    fn withdraw(request: &DNS_SERVICE_REGISTER_REQUEST, replies: &Receiver<u32>) -> bool {
        // SAFETY: `request` is the same registration request that succeeded in `run`.
        let status = unsafe { DnsServiceDeRegister(request, None) };
        if status != DNS_REQUEST_PENDING as u32 {
            tracing::debug!(code = status, "could not withdraw the network announcement");
            return true;
        }
        let confirmed = replies.recv_timeout(REPLY_TIME).is_ok();
        if confirmed {
            tracing::debug!("withdrew the network announcement");
        } else {
            tracing::debug!("Windows did not confirm withdrawing the announcement");
        }
        confirmed
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{avahi, Announcement};
    use std::sync::mpsc::Receiver;

    /// Publishes the service through Avahi's service folder, waits for `stop`, then removes it.
    pub fn run(announcement: &Announcement, stop: &Receiver<()>) {
        let path = std::path::Path::new(avahi::SERVICE_FILE);
        if let Err(err) = std::fs::write(path, avahi::service_xml(announcement)) {
            tracing::warn!(
                error = %err,
                path = %path.display(),
                "could not announce Couchside on the network (is avahi-daemon installed?)"
            );
            return;
        }
        tracing::info!(
            name = %announcement.instance_name(),
            port = announcement.port,
            "announced on the local network through Avahi (Home Assistant can discover it)"
        );
        let _ = stop.recv();
        if let Err(err) = std::fs::remove_file(path) {
            tracing::debug!(error = %err, "could not withdraw the network announcement");
        }
    }
}

/// Avahi service definitions (Linux).
#[cfg(any(not(windows), test))]
mod avahi {
    use super::Announcement;

    /// Avahi publishes every file in this folder and notices changes by itself.
    #[cfg_attr(windows, allow(dead_code))]
    pub const SERVICE_FILE: &str = "/etc/avahi/services/couchside.service";

    /// `s` escaped for XML text.
    fn escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    /// The Avahi service file for `a`.
    pub fn service_xml(a: &Announcement) -> String {
        let txt: String = a
            .txt
            .iter()
            .map(|(k, v)| format!("    <txt-record>{}={}</txt-record>\n", escape(k), escape(v)))
            .collect();
        format!(
            "<?xml version=\"1.0\" standalone=\"no\"?>\n\
             <!DOCTYPE service-group SYSTEM \"avahi-service.dtd\">\n\
             <!-- Written by Couchside while it runs; removed when it stops. -->\n\
             <service-group>\n  <name>{}</name>\n  <service>\n    <type>_couchside._tcp</type>\n    \
             <port>{}</port>\n{txt}  </service>\n</service-group>\n",
            escape(&a.name),
            a.port
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcement_names() {
        let a = Announcement::for_server("ljdesktop", Some(Ipv4Addr::new(192, 168, 1, 20)), 8096);
        assert_eq!(
            a.instance_name(),
            "Couchside on ljdesktop._couchside._tcp.local"
        );
        assert_eq!(a.host, "ljdesktop.local");
        assert!(a.txt.iter().any(|(k, v)| k == "api" && v == "/api/v1"));
    }

    #[test]
    fn avahi_service_file() {
        let a = Announcement::for_server("media&tv", None, 8096);
        let xml = avahi::service_xml(&a);
        assert!(xml.contains("<name>Couchside on media&amp;tv</name>"));
        assert!(xml.contains("<type>_couchside._tcp</type>"));
        assert!(xml.contains("<port>8096</port>"));
        assert!(xml.contains("<txt-record>api=/api/v1</txt-record>"));
        assert!(xml.contains("<txt-record>server=media&amp;tv</txt-record>"));
    }
}
