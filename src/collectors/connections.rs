use crate::types::ZmqConn;

/// Parse /proc/net/tcp to find connections on the given Salt ports.
pub fn collect_connections(publish_port: u16, ret_port: u16) -> Vec<ZmqConn> {
    let mut conns = Vec::new();
    conns.extend(parse_proc_net_tcp("/proc/net/tcp", publish_port, ret_port, 0));
    conns.extend(parse_proc_net_tcp("/proc/net/tcp6", publish_port, ret_port, 0));
    conns
}

/// Like collect_connections but also filters for an API port.
pub fn collect_connections_with_api(publish_port: u16, ret_port: u16, api_port: u16) -> Vec<ZmqConn> {
    let mut conns = Vec::new();
    conns.extend(parse_proc_net_tcp("/proc/net/tcp", publish_port, ret_port, api_port));
    conns.extend(parse_proc_net_tcp("/proc/net/tcp6", publish_port, ret_port, api_port));
    conns
}

fn parse_proc_net_tcp(path: &str, port1: u16, port2: u16, port3: u16) -> Vec<ZmqConn> {
    let mut conns = Vec::new();
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(_) => return conns,
    };

    for line in data.lines().skip(1) {
        // Format: sl  local_address rem_address   st tx_queue:rx_queue tr tm->when retrnsmt
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }

        let local = parts[1];
        let remote = parts[2];
        let st = parts[3];

        let local_port = parse_hex_port(local);
        if local_port != port1 && local_port != port2 && local_port != port3 {
            continue;
        }

        let state = tcp_state_name(u32::from_str_radix(st, 16).unwrap_or(0));

        // Parse remote address
        let (remote_ip, remote_port) = if let Some(colon_pos) = remote.rfind(':') {
            let ip_hex = &remote[..colon_pos];
            let port_hex = &remote[colon_pos + 1..];
            let ip = hex_to_ipv4(ip_hex);
            let port = u16::from_str_radix(port_hex, 16).unwrap_or(0);
            (ip, port)
        } else {
            ("0.0.0.0".to_string(), 0)
        };

        conns.push(ZmqConn {
            local_port,
            remote_addr: remote_ip,
            remote_port,
            state,
        });
    }

    conns
}

fn hex_to_ipv4(hex: &str) -> String {
    // Little-endian hex: 0100007F → 127.0.0.1
    let bytes = hex.as_bytes();
    if bytes.len() < 8 {
        return "0.0.0.0".to_string();
    }
    let b0 = u8::from_str_radix(&hex[6..8], 16).unwrap_or(0);
    let b1 = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    let b2 = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b3 = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    format!("{}.{}.{}.{}", b0, b1, b2, b3)
}

fn parse_hex_port(addr: &str) -> u16 {
    if let Some(colon_pos) = addr.rfind(':') {
        u16::from_str_radix(&addr[colon_pos + 1..], 16).unwrap_or(0)
    } else {
        0
    }
}

fn tcp_state_name(st: u32) -> String {
    match st {
        1 => "ESTABLISHED".to_string(),
        2 => "SYN_SENT".to_string(),
        3 => "SYN_RECV".to_string(),
        4 => "FIN_WAIT1".to_string(),
        5 => "FIN_WAIT2".to_string(),
        6 => "TIME_WAIT".to_string(),
        7 => "CLOSE".to_string(),
        8 => "CLOSE_WAIT".to_string(),
        9 => "LAST_ACK".to_string(),
        10 => "LISTEN".to_string(),
        11 => "CLOSING".to_string(),
        _ => format!("UNKNOWN({})", st),
    }
}
