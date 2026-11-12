use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Validation {
    pub valid: bool,
    pub message: String,
    pub protocols: Vec<String>,
    pub inbounds: Vec<String>,
    pub requires_elevation: bool,
}

pub fn parse(text: &str) -> Result<Value, String> {
    if text.len() > MAX_CONFIG_BYTES {
        return Err("Configuration must be smaller than 1 MB.".into());
    }
    if text.trim().is_empty() {
        return Err("Paste your Xray JSON configuration to get started.".into());
    }
    let config: Value = json5::from_str(text)
        .map_err(|_| "Invalid JSON. Check brackets, commas and quotation marks.".to_string())?;
    if !config.is_object() {
        return Err("Configuration must be a JSON object.".into());
    }
    for key in ["inbounds", "outbounds"] {
        let entries = config[key]
            .as_array()
            .filter(|entries| !entries.is_empty())
            .ok_or_else(|| format!("Add at least one entry to the “{key}” array."))?;
        for entry in entries {
            if !entry.is_object() || entry["protocol"].as_str().is_none_or(str::is_empty) {
                return Err(format!("Each entry in “{key}” needs a protocol."));
            }
            for field in ["settings", "streamSettings"] {
                if entry.get(field).is_some_and(|value| !value.is_object()) {
                    return Err(format!("{key} {field} must be a JSON object."));
                }
            }
        }
    }
    for key in [
        "log",
        "dns",
        "routing",
        "policy",
        "api",
        "transport",
        "stats",
    ] {
        if config.get(key).is_some_and(|value| !value.is_object()) {
            return Err(format!("“{key}” must be a JSON object."));
        }
    }
    Ok(config)
}

pub fn validate(text: &str) -> Validation {
    match parse(text) {
        Ok(config) => Validation {
            valid: true,
            message: "Ready. Your configured inbounds will be used without modification.".into(),
            protocols: protocols(&config, "outbounds"),
            inbounds: protocols(&config, "inbounds"),
            requires_elevation: requires_elevation(&config),
        },
        Err(message) => Validation {
            valid: false,
            message,
            protocols: vec![],
            inbounds: vec![],
            requires_elevation: false,
        },
    }
}

fn protocols(config: &Value, field: &str) -> Vec<String> {
    config[field]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["protocol"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn privileged_port(value: &Value) -> bool {
    if let Some(port) = value.as_u64() {
        return port > 0 && port < 1024;
    }
    value.as_str().is_some_and(|ports| {
        ports.split(',').any(|range| {
            range
                .trim()
                .split('-')
                .next()
                .and_then(|port| port.parse::<u16>().ok())
                .is_some_and(|port| port > 0 && port < 1024)
        })
    })
}

pub fn requires_elevation(config: &Value) -> bool {
    config["inbounds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|inbound| {
            inbound["protocol"]
                .as_str()
                .is_some_and(|protocol| protocol.eq_ignore_ascii_case("tun"))
                || (cfg!(unix) && privileged_port(&inbound["port"]))
                || inbound
                    .pointer("/streamSettings/sockopt/tproxy")
                    .and_then(Value::as_str)
                    .is_some_and(|mode| !mode.is_empty() && mode != "off")
        })
        || config["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|outbound| {
                outbound
                    .pointer("/streamSettings/sockopt/mark")
                    .and_then(Value::as_u64)
                    .is_some_and(|mark| mark != 0)
            })
}

pub fn runtime_config(input: &Value) -> Value {
    let mut config = input.clone();
    // Only logging is redirected: an imported path must not cause a privileged
    // process to overwrite an arbitrary file. All networking settings stay intact.
    config["log"] = json!({"loglevel":"info", "access":"none", "error":""});
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Value {
        parse(r#"{
        // A normal local proxy: no TUN and no administrator prompt.
        "inbounds":[{"tag":"local","listen":"127.0.0.1","port":1080,"protocol":"socks","settings":{"udp":true}}],
        "outbounds":[{"protocol":"freedom","tag":"direct"}],
        "routing":{"rules":[{"inboundTag":["local"],"outboundTag":"direct"}]},
    }"#).unwrap()
    }

    #[test]
    fn accepts_commented_config_and_reports_its_actual_inbounds() {
        let validation = validate(&input().to_string());
        assert!(validation.valid);
        assert_eq!(validation.inbounds, vec!["socks"]);
        assert_eq!(validation.protocols, vec!["freedom"]);
        assert!(!validation.requires_elevation);
    }
    #[test]
    fn preserves_all_networking_settings_including_inbounds_and_api() {
        let mut config = input();
        config["api"] = json!({"tag":"api", "services":["StatsService"]});
        let mut expected = config.clone();
        expected["log"] = json!({"loglevel":"info", "access":"none", "error":""});
        assert_eq!(runtime_config(&config), expected);
        assert_eq!(config["inbounds"][0]["protocol"], "socks");
    }
    #[test]
    fn does_not_add_tun_routes_dns_or_socket_options() {
        let config = runtime_config(&input());
        assert_eq!(config["inbounds"], input()["inbounds"]);
        assert_eq!(config["outbounds"], input()["outbounds"]);
        assert_eq!(config["routing"], input()["routing"]);
        assert_eq!(config["dns"], Value::Null);
    }
    #[test]
    fn native_tun_is_opt_in_and_requires_authorization() {
        let mut config = input();
        config["inbounds"] = json!([{"tag":"vpn", "protocol":"tun", "settings":{"name":"user-tun","gateway":["172.28.233.1/30"],"autoSystemRoutingTable":["0.0.0.0/0"]}}]);
        let validation = validate(&config.to_string());
        assert_eq!(validation.inbounds, vec!["tun"]);
        assert!(validation.requires_elevation);
        assert_eq!(runtime_config(&config)["inbounds"], config["inbounds"]);
    }
    #[test]
    fn detects_privileged_ports_and_ranges() {
        assert!(privileged_port(&json!(443)));
        assert!(privileged_port(&json!("80,1080-1090")));
        assert!(privileged_port(&json!("1000-1100")));
        assert!(!privileged_port(&json!(1080)));
        assert!(!privileged_port(&json!(0)));
    }
    #[test]
    fn reports_missing_inbounds_instead_of_inventing_one() {
        assert_eq!(
            validate(r#"{"outbounds":[{"protocol":"freedom"}]}"#).message,
            "Add at least one entry to the “inbounds” array."
        );
        assert_eq!(
            validate("").message,
            "Paste your Xray JSON configuration to get started."
        );
        assert!(validate("not json").message.starts_with("Invalid JSON"));
        assert!(validate(&" ".repeat(MAX_CONFIG_BYTES + 1))
            .message
            .contains("1 MB"));
    }
    #[test]
    fn safely_redirects_privileged_file_logs() {
        let mut config = input();
        config["log"] = json!({"access":"/etc/test", "error":"/etc/test"});
        assert_eq!(
            runtime_config(&config)["log"],
            json!({"loglevel":"info", "access":"none", "error":""})
        );
    }
}
