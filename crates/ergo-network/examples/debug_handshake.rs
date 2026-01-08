//! Debug handshake tool - connects to a peer and displays handshake exchange
//!
//! Usage: cargo run --example debug_handshake -- <ip:port>
//! Example: cargo run --example debug_handshake -- 192.168.1.137:9056

use ergo_network::{HandshakeData, PROTOCOL_VERSION};
use std::env;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();
    let addr = if args.len() > 1 {
        &args[1]
    } else {
        "192.168.1.137:9056"
    };

    println!("Connecting to {}...", addr);

    match TcpStream::connect_timeout(
        &addr.to_socket_addrs().unwrap().next().unwrap(),
        Duration::from_secs(10),
    ) {
        Ok(mut stream) => {
            println!("Connected! Sending handshake...");

            stream
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(10)))
                .unwrap();

            // Build handshake using the library
            let session_id: u64 = rand::random();
            let handshake = HandshakeData::new(
                "ergo-rust-node".to_string(),
                PROTOCOL_VERSION,
                "debug-test".to_string(),
            )
            .with_mainnet_features(session_id);

            let handshake_bytes = handshake.serialize();
            println!("Handshake bytes ({}):", handshake_bytes.len());
            println!("{}", hex::encode(&handshake_bytes));

            match stream.write_all(&handshake_bytes) {
                Ok(_) => {
                    println!("Handshake sent, flushing...");
                    stream.flush().unwrap();

                    // Read response
                    let mut buf = [0u8; 2048];
                    println!("Reading response...");
                    match stream.read(&mut buf) {
                        Ok(n) => {
                            println!("Received {} bytes:", n);
                            println!("{}", hex::encode(&buf[..n]));

                            // Try to parse
                            if n > 0 {
                                match HandshakeData::parse(&buf[..n]) {
                                    Ok(their_handshake) => {
                                        println!("\nParsed handshake successfully:");
                                        println!("  Agent: {}", their_handshake.agent_name);
                                        println!("  Version: {:?}", their_handshake.version);
                                        println!("  Peer name: {}", their_handshake.peer_name);
                                        if let Some(ref addr) = their_handshake.declared_addr {
                                            println!(
                                                "  Declared address: {}:{}",
                                                addr.ip, addr.port
                                            );
                                        }
                                        println!("  Features: {}", their_handshake.features.len());
                                        for feature in &their_handshake.features {
                                            println!("    {:?}", feature);
                                        }

                                        // Now try to read a framed message
                                        println!("\nWaiting for framed message...");
                                        match stream.read(&mut buf) {
                                            Ok(n) if n > 0 => {
                                                println!("Received {} more bytes:", n);
                                                println!("{}", hex::encode(&buf[..n]));
                                                if n >= 13 {
                                                    println!("  Magic: {:?}", &buf[0..4]);
                                                    println!("  Type: {}", buf[4]);
                                                    let len = u32::from_be_bytes([
                                                        buf[5], buf[6], buf[7], buf[8],
                                                    ]);
                                                    println!("  Length: {}", len);
                                                    println!("  Checksum: {:?}", &buf[9..13]);
                                                }
                                            }
                                            Ok(_) => println!("Connection closed after handshake"),
                                            Err(e) => println!("Read error: {}", e),
                                        }
                                    }
                                    Err(e) => {
                                        println!("Failed to parse handshake: {}", e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            println!("Read error: {}", e);
                        }
                    }
                }
                Err(e) => {
                    println!("Write error: {}", e);
                }
            }
        }
        Err(e) => {
            println!("Connection error: {}", e);
        }
    }
}
