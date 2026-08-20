//@ network
//@ run-flags: --test-threads 1 --show-output

use std::io::{Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::time::Duration;

#[test]
fn public_network_reaches_an_http_server() {
    let address = ("example.com", 80)
        .to_socket_addrs()
        .unwrap()
        .find(std::net::SocketAddr::is_ipv4)
        .unwrap();
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10)).unwrap();
    stream
        .write_all(b"GET / HTTP/1.0\r\nHost: example.com\r\nConnection: close\r\n\r\n")
        .unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    assert!(response.starts_with("HTTP/1."), "unexpected response: {response:?}");
    println!("public network available inside Microsandbox");
}
