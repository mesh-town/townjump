//! SACK tests: block validator (D6), block area in an incoming ACK, `on_sack`.
use super::*;
use crate::native::header::{FLAG_DATA, FLAG_SACK, Header, SackRange};
use crate::native::stream::StreamInner;
use crate::native::stream::test_support::{stream_inner, with_sent};
use std::time::Duration;

mod sequence_difference {
    use super::*;

    #[test]
    fn number_after_wraparound_is_ahead() {
        assert_eq!(seq_diff(1, u32::MAX), 2);
    }

    #[test]
    fn number_before_wraparound_is_behind() {
        assert_eq!(seq_diff(u32::MAX, 1), -2);
    }

    #[test]
    fn equal_numbers_have_zero_difference() {
        assert_eq!(seq_diff(7, 7), 0);
    }
}

mod block_validation {
    use super::*;

    mod acked_0_next_seq_6 {
        use super::*;

        #[test]
        fn block_inside_sent_range_is_accepted() {
            assert!(sack_block_valid(0, 6, 0, 2), "[0,2)");
        }

        #[test]
        fn block_ending_exactly_at_seq_is_accepted() {
            assert!(sack_block_valid(0, 6, 0, 6), "[0,6)");
        }

        #[test]
        fn block_ending_above_seq_is_rejected() {
            assert!(!sack_block_valid(0, 6, 0, 7), "[0,7)");
        }

        #[test]
        fn empty_block_is_rejected() {
            assert!(!sack_block_valid(0, 6, 0, 0), "[0,0)");
        }

        #[test]
        fn block_with_end_below_start_is_rejected() {
            assert!(!sack_block_valid(0, 6, 5, 0), "[5,0)");
        }
    }

    mod acked_2 {
        use super::*;

        #[test]
        fn block_starting_below_acked_is_rejected() {
            assert!(!sack_block_valid(2, 6, 1, 3), "[1,3)");
        }

        #[test]
        fn block_starting_at_acked_is_accepted() {
            assert!(sack_block_valid(2, 6, 2, 4), "[2,4)");
        }
    }

    mod across_wraparound {
        use super::*;

        const ACKED: u32 = u32::MAX - 1;
        const SEQ: u32 = 2;

        #[test]
        fn block_crossing_2_pow_32_is_accepted() {
            assert!(sack_block_valid(ACKED, SEQ, u32::MAX, 1), "[MAX,1)");
        }

        #[test]
        fn block_running_backwards_over_2_pow_32_is_rejected() {
            assert!(!sack_block_valid(ACKED, SEQ, 1, u32::MAX), "[1,MAX)");
        }
    }
}

mod block_area_length {
    use super::*;

    mod data_offset_zero {
        use super::*;

        #[test]
        fn whole_payload_is_blocks() {
            assert_eq!(sack_block_bytes(0, 16), 16);
        }

        #[test]
        fn tail_not_multiple_of_8_is_cut() {
            assert_eq!(sack_block_bytes(0, 20), 16);
        }

        #[test]
        fn empty_payload_has_no_blocks() {
            assert_eq!(sack_block_bytes(0, 0), 0);
        }
    }

    mod data_offset_inside_payload {
        use super::*;

        #[test]
        fn blocks_end_at_data_offset() {
            assert_eq!(sack_block_bytes(8, 24), 8);
        }
    }

    mod data_offset_equals_payload_len {
        use super::*;

        #[test]
        fn old_rust_format_reads_whole_payload() {
            assert_eq!(sack_block_bytes(16, 16), 16);
        }
    }

    mod data_offset_beyond_payload {
        use super::*;

        #[test]
        fn whole_payload_is_read_without_tail() {
            assert_eq!(sack_block_bytes(255, 20), 16);
        }
    }
}

mod incoming_sack_blocks {
    use super::*;

    const TWO_BLOCKS: [(u32, u32); 2] = [(4, 6), (7, 8)];

    mod js_format {
        use super::*;

        #[test]
        fn two_blocks_after_header_are_read() {
            // given: FLAG_SACK, data_offset 0, 16 bytes of blocks
            let (header, data) = ack_packet_with(FLAG_SACK, 0, &blocks_bytes(&TWO_BLOCKS));
            // when
            let sacks = incoming_sacks(&header, &data);
            // then
            assert_eq!(sacks, ranges(&TWO_BLOCKS));
        }

        #[test]
        fn tail_of_4_bytes_is_dropped() {
            // given: two blocks and 4 extra bytes
            let mut payload = blocks_bytes(&TWO_BLOCKS);
            payload.extend_from_slice(&[9, 9, 9, 9]);
            let (header, data) = ack_packet_with(FLAG_SACK, 0, &payload);
            // when
            let sacks = incoming_sacks(&header, &data);
            // then
            assert_eq!(sacks, ranges(&TWO_BLOCKS));
        }

        #[test]
        fn sixty_blocks_are_all_read() {
            // given: 60 blocks (the limit of 50 applies on write, not on read)
            let pairs: Vec<(u32, u32)> = (0..60).map(|i| (i * 2, i * 2 + 1)).collect();
            let (header, data) = ack_packet_with(FLAG_SACK, 0, &blocks_bytes(&pairs));
            // when
            let sacks = incoming_sacks(&header, &data);
            // then
            assert_eq!(sacks, ranges(&pairs));
        }

        #[test]
        fn packet_without_payload_has_no_blocks() {
            let (header, data) = ack_packet_with(FLAG_SACK, 0, &[]);

            assert!(incoming_sacks(&header, &data).is_empty());
        }
    }

    mod old_rust_format {
        use super::*;

        #[test]
        fn data_offset_equal_to_payload_len_still_reads_blocks() {
            // given: data_offset = 16 = length of the blocks
            let (header, data) = ack_packet_with(FLAG_SACK, 16, &blocks_bytes(&TWO_BLOCKS));
            // when
            let sacks = incoming_sacks(&header, &data);
            // then
            assert_eq!(sacks, ranges(&TWO_BLOCKS));
        }
    }

    mod packet_without_sack_flag {
        use super::*;

        #[test]
        fn data_offset_bytes_are_not_blocks() {
            // given: DATA with data_offset > 0 (e.g. an MTU probe), no SACK flag
            let (header, data) = ack_packet_with(FLAG_DATA, 16, &blocks_bytes(&TWO_BLOCKS));
            // when
            let sacks = incoming_sacks(&header, &data);
            // then
            assert!(sacks.is_empty(), "no blocks without FLAG_SACK: {sacks:?}");
        }
    }
}

mod applying_sack {
    use super::*;

    mod six_packets_sent {
        use super::*;

        #[test]
        fn only_valid_blocks_mark_packets() {
            // given: seq 0..5 sent, next_seq = 6
            let mut inner = sent_stream(0, 6);
            // when: [0,2) is valid, [4,9) goes past next_seq
            let marked = seqs(&inner.on_sack(&ranges(&[(0, 2), (4, 9)])));
            // then
            assert_eq!(sacked_seqs(&inner), vec![0, 1], "marked in outgoing");
            assert_eq!(marked, vec![0, 1], "returned as acked");
        }

        #[test]
        fn block_with_end_below_start_is_skipped_promptly() {
            // given
            let inner = sent_stream(0, 6);
            // when: [5,0) without validation would mean walking ~2^32 numbers
            let (inner, marked) = on_sack_within_deadline(inner, ranges(&[(5, 0)]));
            // then
            assert!(marked.is_empty(), "nothing acked: {marked:?}");
            assert!(sacked_seqs(&inner).is_empty(), "nothing marked in outgoing");
        }

        #[test]
        fn repeated_block_returns_nothing_new() {
            // given: block [0,2) already counted
            let mut inner = sent_stream(0, 6);
            inner.on_sack(&ranges(&[(0, 2)]));
            // when
            let again = inner.on_sack(&ranges(&[(0, 2)]));
            // then
            assert!(again.is_empty(), "a repeat gives no new AckedRateInfo");
            assert_eq!(sacked_seqs(&inner), vec![0, 1]);
        }
    }

    mod four_packets_across_wraparound {
        use super::*;

        #[test]
        fn block_crossing_2_pow_32_marks_both_sides() {
            // given: MAX-1, MAX, 0, 1 sent; remote_acked = MAX-1, next_seq = 2
            let mut inner = sent_stream(u32::MAX - 1, 4);
            // when
            let marked = seqs(&inner.on_sack(&ranges(&[(u32::MAX, 1)])));
            // then
            assert_eq!(sacked_seqs(&inner), vec![0, u32::MAX]);
            assert_eq!(marked, vec![0, u32::MAX]);
        }
    }
}

// ── setup and helpers ────────────────────────────────────────────────────────

/// A stream with `count` sent DATA, starting at `first`.
fn sent_stream(first: u32, count: u32) -> StreamInner {
    let mut inner = stream_inner();
    with_sent(&mut inner, first, count);
    inner
}

/// A whole packet (header + `payload`) and its parsed header.
fn ack_packet_with(type_flags: u8, data_offset: u8, payload: &[u8]) -> (Header, Vec<u8>) {
    let header = Header {
        type_flags,
        data_offset,
        remote_id: 5,
        recv_window: 0,
        seq: 0,
        ack: 0,
    };
    let mut data = header.encode().to_vec();
    data.extend_from_slice(payload);
    (header, data)
}

/// Blocks as the sender writes them: pairs `start`, `end` of 4 bytes LE.
fn blocks_bytes(pairs: &[(u32, u32)]) -> Vec<u8> {
    pairs
        .iter()
        .flat_map(|&(start, end)| [start.to_le_bytes(), end.to_le_bytes()].concat())
        .collect()
}

fn ranges(pairs: &[(u32, u32)]) -> Vec<SackRange> {
    pairs
        .iter()
        .map(|&(start, end)| SackRange { start, end })
        .collect()
}

/// Numbers of acked packets in ascending order.
fn seqs<T>(info: &[(u32, T)]) -> Vec<u32> {
    let mut seqs: Vec<u32> = info.iter().map(|(seq, _)| *seq).collect();
    seqs.sort_unstable();
    seqs
}

/// Numbers of `outgoing` packets marked as received by SACK, in ascending order.
fn sacked_seqs(inner: &StreamInner) -> Vec<u32> {
    let mut sacked: Vec<u32> = inner
        .outgoing
        .iter()
        .filter(|(_, packet)| packet.sacked)
        .map(|(seq, _)| seq.0)
        .collect();
    sacked.sort_unstable();
    sacked
}

/// `on_sack` in a separate thread: a hang fails the test by timeout instead of hanging the run.
fn on_sack_within_deadline(
    mut inner: StreamInner,
    blocks: Vec<SackRange>,
) -> (StreamInner, Vec<u32>) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let marked = seqs(&inner.on_sack(&blocks));
        let _ = tx.send((inner, marked));
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("on_sack did not return within 5 s: invalid block not filtered")
}
