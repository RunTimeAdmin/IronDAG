//! Minimal STUN client for public IP discovery (RFC 5389).
//! Used when --advertise is not set to improve NAT traversal.

use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::time::{timeout, Duration};

const STUN_MAGIC: u32 = 0x2112_A442;
const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// Discover public IP:port via STUN (e.g. stun.l.google.com:19302).
/// Returns e.g. "203.0.113.1:8080" (public IP + listen_port) or None on failure/timeout.
pub async fn discover_public_addr(listen_port: u16) -> Option<String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host("stun.l.google.com:19302")
        .await
        .ok()?
        .collect();
    let server = addrs.into_iter().next()?;

    let socket = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    socket.connect(server).await.ok()?;

    // STUN Binding Request: type=0x0001, length=0, magic=0x2112A442, 12-byte transaction ID
    let mut tx_id = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut tx_id);
    let mut req = Vec::with_capacity(20);
    req.extend_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    req.extend_from_slice(&0u16.to_be_bytes());
    req.extend_from_slice(&STUN_MAGIC.to_be_bytes());
    req.extend_from_slice(&tx_id);

    let _ = socket.send(&req).await.ok()?;
    let mut buf = [0u8; 256];
    let recv = match timeout(Duration::from_secs(3), socket.recv_from(&mut buf)).await {
        Ok(Ok((n, _))) => n,
        _ => return None,
    };
    let n = recv;
    if n < 20 {
        return None;
    }
    let msg_type = u16::from_be_bytes([buf[0], buf[1]]);
    if msg_type != STUN_BINDING_RESPONSE {
        return None;
    }
    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if 20 + len > n {
        return None;
    }
    // Parse attributes (after 20-byte header)
    let mut i = 20;
    while i + 4 <= 20 + len {
        let attr_type = u16::from_be_bytes([buf[i], buf[i + 1]]);
        let attr_len = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as usize;
        i += 4;
        if i + attr_len > 20 + len {
            break;
        }
        if attr_type == ATTR_XOR_MAPPED_ADDRESS && attr_len >= 8 {
            // XOR-MAPPED-ADDRESS: 1 reserved, 1 family (0x01=IPv4), 2 port (xored), 4 addr (xored)
            let family = buf[i + 1];
            if family == 0x01 {
                let xaddr = u32::from_be_bytes([buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]]);
                let addr = xaddr ^ STUN_MAGIC;
                let ip = std::net::Ipv4Addr::from(addr);
                return Some(format!("{}:{}", ip, listen_port));
            }
            // IPv6 would need 20 bytes; skip for simplicity
        }
        i += attr_len;
        if attr_len % 4 != 0 {
            i += 4 - (attr_len % 4);
        }
    }
    None
}
