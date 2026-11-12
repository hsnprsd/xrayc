use serde_json::json;
use std::{env, fs, path::PathBuf};
use xrayc_service::config::{parse, runtime_config};

fn main() {
    let dir = PathBuf::from(env::args().nth(1).expect("output directory"));
    fs::create_dir_all(&dir).unwrap();
    let input = parse(&json!({
        "inbounds":[{"tag":"user-tun", "protocol":"tun", "settings":{
            "name":if cfg!(target_os = "macos") { "utun10" } else { "xrayc0" },
            "gateway":["172.28.233.1/30", "fd75:7872:6179::1/126"],
            "autoSystemRoutingTable":["0.0.0.0/1", "128.0.0.0/1", "::/1", "8000::/1"],
            "autoOutboundsInterface":"auto"
        }}],
        "outbounds":[{"protocol":"socks", "settings":{"servers":[{"address":"127.0.0.1", "port":19080}]}}]
    }).to_string()).unwrap();
    fs::write(
        dir.join("xray.json"),
        serde_json::to_vec_pretty(&runtime_config(&input)).unwrap(),
    )
    .unwrap();
    let mut socks = input.clone();
    socks["inbounds"] = json!([{"tag":"user-socks", "protocol":"socks", "listen":"127.0.0.1", "port":19081, "settings":{"udp":true}}]);
    fs::write(
        dir.join("socks.json"),
        serde_json::to_vec_pretty(&runtime_config(&socks)).unwrap(),
    )
    .unwrap();
}
