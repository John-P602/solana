//! The `recvmmsg` module provides a nonblocking recvmmsg() API implementation

use {
    crate::{
        packet::{Meta, Packet},
        recvmmsg::NUM_RCVMMSGS,
    },
    std::{cmp, io},
    tokio::net::UdpSocket,
};

pub async fn recv_mmsg(
    socket: &UdpSocket,
    packets: &mut [Packet],
) -> io::Result</*num packets:*/ usize> {
    debug_assert!(packets.iter().all(|pkt| pkt.meta == Meta::default()));
    let count = cmp::min(NUM_RCVMMSGS, packets.len());
    socket.readable().await?;
    let mut i = 0;
    for p in packets.iter_mut().take(count) {
        p.meta.size = 0;
        match socket.try_recv_from(&mut p.data) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                break;
            }
            Err(e) => {
                return Err(e);
            }
            Ok((nrecv, from)) => {
                p.meta.size = nrecv;
                p.meta.set_addr(&from);
            }
        }
        i += 1;
    }
    Ok(i)
}

#[cfg(test)]
mod tests {
    use {
        crate::{nonblocking::recvmmsg::*, packet::PACKET_DATA_SIZE},
        std::{net::SocketAddr, time::Instant},
        tokio::net::UdpSocket,
    };

    type TestConfig = (UdpSocket, SocketAddr, UdpSocket, SocketAddr);

    async fn test_setup_reader_sender(ip_str: &str) -> io::Result<TestConfig> {
        let reader = UdpSocket::bind(ip_str).await?;
        let addr = reader.local_addr()?;
        let sender = UdpSocket::bind(ip_str).await?;
        let saddr = sender.local_addr()?;
        Ok((reader, addr, sender, saddr))
    }

    const TEST_NUM_MSGS: usize = 32;

    async fn test_one_iter((reader, addr, sender, saddr): TestConfig) {
        let sent = TEST_NUM_MSGS - 1;
        for _ in 0..sent {
            let data = [0; PACKET_DATA_SIZE];
            sender.send_to(&data[..], &addr).await.unwrap();
        }

        let mut packets = vec![Packet::default(); TEST_NUM_MSGS];
        let recv = recv_mmsg(&reader, &mut packets[..]).await.unwrap();
        assert_eq!(sent, recv);
        for packet in packets.iter().take(recv) {
            assert_eq!(packet.meta.size, PACKET_DATA_SIZE);
            assert_eq!(packet.meta.addr(), saddr);
        }
    }

    #[tokio::test]
    async fn test_recv_mmsg_one_iter() {
        test_one_iter(test_setup_reader_sender("127.0.0.1:0").await.unwrap()).await;

        match test_setup_reader_sender("::1:0").await {
            Ok(config) => test_one_iter(config).await,
            Err(e) => warn!("Failed to configure IPv6: {:?}", e),
        }
    }

    async fn test_multi_iter((reader, addr, sender, saddr): TestConfig) {
        let sent = TEST_NUM_MSGS + 10;
        for _ in 0..sent {
            let data = [0; PACKET_DATA_SIZE];
            sender.send_to(&data[..], &addr).await.unwrap();
        }

        let mut packets = vec![Packet::default(); TEST_NUM_MSGS];
        let recv = recv_mmsg(&reader, &mut packets[..]).await.unwrap();
        assert_eq!(TEST_NUM_MSGS, recv);
        for packet in packets.iter().take(recv) {
            assert_eq!(packet.meta.size, PACKET_DATA_SIZE);
            assert_eq!(packet.meta.addr(), saddr);
        }

        packets
            .iter_mut()
            .for_each(|pkt| pkt.meta = Meta::default());
        let recv = recv_mmsg(&reader, &mut packets[..]).await.unwrap();
        assert_eq!(sent - TEST_NUM_MSGS, recv);
        for packet in packets.iter().take(recv) {
            assert_eq!(packet.meta.size, PACKET_DATA_SIZE);
            assert_eq!(packet.meta.addr(), saddr);
        }
    }

    #[tokio::test]
    async fn test_recv_mmsg_multi_iter() {
        test_multi_iter(test_setup_reader_sender("127.0.0.1:0").await.unwrap()).await;

        match test_setup_reader_sender("::1:0").await {
            Ok(config) => test_multi_iter(config).await,
            Err(e) => warn!("Failed to configure IPv6: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_recv_mmsg_multi_iter_timeout() {
        let reader = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let addr = reader.local_addr().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let saddr = sender.local_addr().unwrap();
        let sent = TEST_NUM_MSGS;
        for _ in 0..sent {
            let data = [0; PACKET_DATA_SIZE];
            sender.send_to(&data[..], &addr).await.unwrap();
        }

        let start = Instant::now();
        let mut packets = vec![Packet::default(); TEST_NUM_MSGS];
        let recv = recv_mmsg(&reader, &mut packets[..]).await.unwrap();
        assert_eq!(TEST_NUM_MSGS, recv);
        for packet in packets.iter().take(recv) {
            assert_eq!(packet.meta.size, PACKET_DATA_SIZE);
            assert_eq!(packet.meta.addr(), saddr);
        }

        packets
            .iter_mut()
            .for_each(|pkt| pkt.meta = Meta::default());
        let _recv = recv_mmsg(&reader, &mut packets[..]).await;
        assert!(start.elapsed().as_secs() < 5);
    }

    #[tokio::test]
    async fn test_recv_mmsg_multi_addrs() {
        let reader = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let addr = reader.local_addr().unwrap();

        let sender1 = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let saddr1 = sender1.local_addr().unwrap();
        let sent1 = TEST_NUM_MSGS - 1;

        let sender2 = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let saddr2 = sender2.local_addr().unwrap();
        let sent2 = TEST_NUM_MSGS + 1;

        for _ in 0..sent1 {
            let data = [0; PACKET_DATA_SIZE];
            sender1.send_to(&data[..], &addr).await.unwrap();
        }

        for _ in 0..sent2 {
            let data = [0; PACKET_DATA_SIZE];
            sender2.send_to(&data[..], &addr).await.unwrap();
        }

        let mut packets = vec![Packet::default(); TEST_NUM_MSGS];

        let recv = recv_mmsg(&reader, &mut packets[..]).await.unwrap();
        assert_eq!(TEST_NUM_MSGS, recv);
        for packet in packets.iter().take(sent1) {
            assert_eq!(packet.meta.size, PACKET_DATA_SIZE);
            assert_eq!(packet.meta.addr(), saddr1);
        }
        for packet in packets.iter().skip(sent1).take(recv - sent1) {
            assert_eq!(packet.meta.size, PACKET_DATA_SIZE);
            assert_eq!(packet.meta.addr(), saddr2);
        }

        packets
            .iter_mut()
            .for_each(|pkt| pkt.meta = Meta::default());
        let recv = recv_mmsg(&reader, &mut packets[..]).await.unwrap();
        assert_eq!(sent1 + sent2 - TEST_NUM_MSGS, recv);
        for packet in packets.iter().take(recv) {
            assert_eq!(packet.meta.size, PACKET_DATA_SIZE);
            assert_eq!(packet.meta.addr(), saddr2);
        }
    }
}
