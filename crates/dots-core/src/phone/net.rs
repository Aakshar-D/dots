//! Which addresses of this PC the phone listener may bind (phone view spec section 3): the
//! private ones of the home network, never a public one, never loopback.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, UdpSocket};

/// Whether `ip` belongs to a private network, so a listener on it is reachable only from that
/// network: IPv4 10/8, 172.16/12 and 192.168/16 (RFC 1918), IPv4 link-local 169.254/16, IPv6
/// unique local fc00::/7 and IPv6 link-local fe80::/10. Everything else is refused: public
/// addresses, loopback, unspecified, broadcast, multicast, documentation ranges and the
/// carrier-grade NAT range 100.64/10 (it is shared with strangers on the same carrier, and VPNs
/// such as Tailscale use it). An IPv4 address mapped into IPv6 is judged as the IPv4 address.
pub fn is_private_lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4_private(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4_private(v4),
            None => v6_private(v6),
        },
    }
}

fn v4_private(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}

fn v6_private(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    // fc00::/7 (unique local) or fe80::/10 (link-local).
    (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
}

/// Whether a phone's browser can open `ip` in a URL. An IPv6 link-local address needs a zone
/// (`%12`) that browsers do not accept in a URL, so it is left out even though it is private.
fn usable_in_url(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(_) => true,
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
    }
}

/// The addresses the listener binds, from `found` (every unicast address of the interfaces
/// that are up): the private ones a browser can open, without repeats, IPv4 first, the
/// address of the default route (`primary`) at the very front when it is one of them.
pub fn pick(found: &[IpAddr], primary: Option<IpAddr>) -> Vec<IpAddr> {
    let mut picked: Vec<IpAddr> = Vec::new();
    for ip in found {
        if is_private_lan(*ip) && usable_in_url(*ip) && !picked.contains(ip) {
            picked.push(*ip);
        }
    }
    picked.sort_by_key(|ip| (Some(*ip) != primary, ip.is_ipv6()));
    picked
}

/// The private addresses of this PC that the listener may bind now (see `pick`).
pub fn lan_addresses() -> Vec<IpAddr> {
    pick(&interface_addresses(), primary_address())
}

/// The local address the system would send from to reach the internet: the address of the
/// default route, which is the Wi-Fi or Ethernet a phone at home is on. A UDP "connect" sends
/// nothing; it only asks the routing table. `None` with no default route.
fn primary_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // TEST-NET-3 (RFC 5737): never answered, and nothing is sent anyway.
    socket.connect((Ipv4Addr::new(203, 0, 113, 1), 9)).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Every unicast address of the network interfaces that are up, except loopback and tunnel
/// interfaces, whose addresses are in their preferred state (an address still being checked
/// for duplicates, or on its way out, is left out).
#[cfg(windows)]
fn interface_addresses() -> Vec<IpAddr> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_FRIENDLY_NAME, GAA_FLAG_SKIP_MULTICAST, IF_TYPE_SOFTWARE_LOOPBACK,
        IF_TYPE_TUNNEL, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{
        IpDadStatePreferred, AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6,
    };

    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    let flags = GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER
        | GAA_FLAG_SKIP_FRIENDLY_NAME;
    // u64 cells keep the buffer aligned for the structures the call writes into it.
    let mut size: u32 = 16 * 1024;
    let mut buf: Vec<u64>;
    let mut tries = 0;
    loop {
        buf = vec![0u64; (size as usize).div_ceil(8)];
        // SAFETY: the buffer holds `size` bytes, as the call is told.
        let rc = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                flags,
                std::ptr::null(),
                buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
                &mut size,
            )
        };
        if rc == 0 {
            break;
        }
        tries += 1;
        if rc != ERROR_BUFFER_OVERFLOW || tries > 3 {
            tracing::warn!("listing the network interfaces failed: error {rc}");
            return Vec::new();
        }
    }
    let mut out = Vec::new();
    let mut adapter = buf.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    // SAFETY: a successful call leaves a linked list of adapters in the buffer, each with a
    // linked list of unicast addresses whose socket addresses are as long as they say; the
    // buffer outlives the walk.
    unsafe {
        while !adapter.is_null() {
            let a = &*adapter;
            let skip = a.OperStatus != IfOperStatusUp
                || a.IfType == IF_TYPE_SOFTWARE_LOOPBACK
                || a.IfType == IF_TYPE_TUNNEL;
            let mut unicast = if skip {
                std::ptr::null_mut()
            } else {
                a.FirstUnicastAddress
            };
            while !unicast.is_null() {
                let u = &*unicast;
                let sa = u.Address.lpSockaddr;
                let len = u.Address.iSockaddrLength as usize;
                if !sa.is_null() && u.DadState == IpDadStatePreferred {
                    match (*sa).sa_family {
                        AF_INET if len >= std::mem::size_of::<SOCKADDR_IN>() => {
                            let v4 = &*sa.cast::<SOCKADDR_IN>();
                            let raw = v4.sin_addr.S_un.S_addr;
                            out.push(IpAddr::V4(Ipv4Addr::from(u32::from_be(raw))));
                        }
                        AF_INET6 if len >= std::mem::size_of::<SOCKADDR_IN6>() => {
                            let v6 = &*sa.cast::<SOCKADDR_IN6>();
                            out.push(IpAddr::V6(Ipv6Addr::from(v6.sin6_addr.u.Byte)));
                        }
                        _ => {}
                    }
                }
                unicast = u.Next;
            }
            adapter = a.Next;
        }
    }
    out
}

/// Elsewhere only the address of the default route is known.
#[cfg(not(windows))]
fn interface_addresses() -> Vec<IpAddr> {
    primary_address().into_iter().collect()
}

/// `ip` as the host part of a URL or a `Host` header: IPv6 in brackets.
pub fn url_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn private_ranges_are_accepted() {
        for s in [
            "10.0.0.1",
            "10.255.255.254",
            "172.16.0.1",
            "172.31.255.1",
            "192.168.1.20",
            "169.254.10.10",
            "fd12:3456:789a::1",
            "fc00::1",
            "fe80::1",
            "::ffff:192.168.1.5",
        ] {
            assert!(is_private_lan(ip(s)), "{s} is private");
        }
    }

    #[test]
    fn public_and_special_addresses_are_refused() {
        for s in [
            "8.8.8.8",
            "1.1.1.1",
            "172.15.255.255",
            "172.32.0.1",
            "192.169.0.1",
            "11.0.0.1",
            "100.64.0.1",
            "100.100.100.100",
            "127.0.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "192.0.2.1",
            "203.0.113.5",
            "::1",
            "::",
            "2001:4860:4860::8888",
            "2a00:1450::1",
            "ff02::1",
            "::ffff:8.8.8.8",
            "fec0::1",
        ] {
            assert!(!is_private_lan(ip(s)), "{s} is refused");
        }
    }

    #[test]
    fn pick_keeps_private_addresses_browsers_can_open_primary_first() {
        let found = [
            ip("8.8.8.8"),
            ip("127.0.0.1"),
            ip("fe80::1"),
            ip("fd00::5"),
            ip("172.20.0.1"),
            ip("192.168.1.20"),
            ip("192.168.1.20"),
            ip("2001:db8::1"),
            ip("100.64.1.2"),
        ];
        assert_eq!(
            pick(&found, Some(ip("192.168.1.20"))),
            vec![ip("192.168.1.20"), ip("172.20.0.1"), ip("fd00::5")]
        );
        // A public default route is never added by being primary.
        assert_eq!(
            pick(&[ip("8.8.4.4")], Some(ip("8.8.4.4"))),
            Vec::<IpAddr>::new()
        );
    }

    #[test]
    fn url_hosts_bracket_ipv6() {
        assert_eq!(url_host(ip("192.168.1.2")), "192.168.1.2");
        assert_eq!(url_host(ip("fd00::5")), "[fd00::5]");
    }
}
