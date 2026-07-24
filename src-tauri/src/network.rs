use reqwest::{Client, ClientBuilder, NoProxy, Proxy};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SystemProxy {
    http: Option<String>,
    https: Option<String>,
    bypass: Vec<String>,
}

pub fn client_builder() -> Result<ClientBuilder, reqwest::Error> {
    let builder = Client::builder();
    if environment_proxy_configured() {
        return Ok(builder);
    }

    apply_system_proxy(builder, platform_proxy())
}

fn environment_proxy_configured() -> bool {
    [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .into_iter()
    .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

fn apply_system_proxy(
    mut builder: ClientBuilder,
    settings: Option<SystemProxy>,
) -> Result<ClientBuilder, reqwest::Error> {
    let Some(settings) = settings else {
        return Ok(builder);
    };
    let no_proxy = no_proxy(&settings.bypass);

    match (&settings.http, &settings.https) {
        (Some(http), Some(https)) if http == https => {
            builder = builder.proxy(Proxy::all(http)?.no_proxy(no_proxy));
        }
        (http, https) => {
            if let Some(http) = http {
                builder = builder.proxy(Proxy::http(http)?.no_proxy(no_proxy.clone()));
            }
            if let Some(https) = https {
                builder = builder.proxy(Proxy::https(https)?.no_proxy(no_proxy));
            }
        }
    }
    Ok(builder)
}

fn no_proxy(entries: &[String]) -> Option<NoProxy> {
    let entries = entries
        .iter()
        .flat_map(|entry| entry.split([';', ',']))
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .flat_map(|entry| {
            if entry.eq_ignore_ascii_case("<local>") {
                vec!["localhost", "127.0.0.1", "::1"]
            } else {
                vec![entry.trim_start_matches('*')]
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    (!entries.is_empty())
        .then(|| NoProxy::from_string(&entries))
        .flatten()
}

fn normalize_proxy_url(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"');
    let normalized = if value.is_empty() {
        return None;
    } else if value.contains("://") {
        value.to_owned()
    } else {
        format!("http://{value}")
    };
    reqwest::Url::parse(&normalized).ok().map(|_| normalized)
}

#[cfg(any(windows, test))]
fn parse_proxy_server(value: &str) -> SystemProxy {
    let mut settings = SystemProxy::default();
    for part in value
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if let Some((kind, address)) = part.split_once('=') {
            match kind.trim().to_ascii_lowercase().as_str() {
                "http" => settings.http = normalize_proxy_url(address),
                "https" => settings.https = normalize_proxy_url(address),
                _ => {}
            }
        } else {
            let proxy = normalize_proxy_url(part);
            settings.http.clone_from(&proxy);
            settings.https = proxy;
        }
    }
    settings
}

#[cfg(windows)]
fn platform_proxy() -> Option<SystemProxy> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    let internet_settings = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enabled = internet_settings
        .get_value::<u32, _>("ProxyEnable")
        .unwrap_or_default();
    if enabled == 0 {
        return None;
    }

    let server = internet_settings
        .get_value::<String, _>("ProxyServer")
        .ok()?;
    let mut settings = parse_proxy_server(&server);
    settings.bypass = internet_settings
        .get_value::<String, _>("ProxyOverride")
        .ok()
        .into_iter()
        .collect();
    (settings.http.is_some() || settings.https.is_some()).then_some(settings)
}

#[cfg(target_os = "macos")]
fn platform_proxy() -> Option<SystemProxy> {
    let output = std::process::Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let text = String::from_utf8(output.stdout).ok()?;
    parse_scutil_proxy(&text)
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn platform_proxy() -> Option<SystemProxy> {
    None
}

#[cfg(any(test, target_os = "macos"))]
fn parse_scutil_proxy(text: &str) -> Option<SystemProxy> {
    fn value<'a>(text: &'a str, name: &str) -> Option<&'a str> {
        text.lines().find_map(|line| {
            let (key, value) = line.trim().split_once(':')?;
            (key.trim() == name).then(|| value.trim())
        })
    }

    fn endpoint(text: &str, prefix: &str) -> Option<String> {
        if value(text, &format!("{prefix}Enable"))? != "1" {
            return None;
        }
        let host = value(text, &format!("{prefix}Proxy"))?;
        let port = value(text, &format!("{prefix}Port"))?;
        normalize_proxy_url(&format!("{host}:{port}"))
    }

    let mut settings = SystemProxy {
        http: endpoint(text, "HTTP"),
        https: endpoint(text, "HTTPS"),
        bypass: Vec::new(),
    };
    let mut in_exceptions = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("ExceptionsList") {
            in_exceptions = true;
            continue;
        }
        if in_exceptions && line == "}" {
            in_exceptions = false;
            continue;
        }
        if in_exceptions && let Some((_, entry)) = line.split_once(':') {
            let entry = entry.trim();
            if !entry.is_empty() {
                settings.bypass.push(entry.to_owned());
            }
        }
    }
    if value(text, "ExcludeSimpleHostnames") == Some("1") {
        settings.bypass.push("<local>".into());
    }

    (settings.http.is_some() || settings.https.is_some()).then_some(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_shared_and_protocol_proxy_values() {
        let shared = parse_proxy_server("127.0.0.1:7890");
        assert_eq!(shared.http.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(shared.https, shared.http);

        let split = parse_proxy_server("http=proxy.local:8080;https=secure.local:8443;socks=skip");
        assert_eq!(split.http.as_deref(), Some("http://proxy.local:8080"));
        assert_eq!(split.https.as_deref(), Some("http://secure.local:8443"));
    }

    #[test]
    fn parses_macos_proxy_and_bypass_settings() {
        let settings = parse_scutil_proxy(
            r#"<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 10.0.0.0/8
  }
  ExcludeSimpleHostnames : 1
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7890
  HTTPSProxy : 127.0.0.1
}"#,
        )
        .unwrap();
        assert_eq!(settings.http.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(settings.https, settings.http);
        assert_eq!(settings.bypass, vec!["*.local", "10.0.0.0/8", "<local>"]);
    }

    #[test]
    fn builds_clients_from_valid_system_proxy_settings() {
        let settings = SystemProxy {
            http: Some("http://127.0.0.1:7890".into()),
            https: Some("http://127.0.0.1:7890".into()),
            bypass: vec!["<local>;*.example.com".into()],
        };
        assert!(
            apply_system_proxy(Client::builder(), Some(settings))
                .and_then(ClientBuilder::build)
                .is_ok()
        );
    }
}
