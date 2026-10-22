// Tests of `udp_frame`: frame `[u32 BE length][body]` — encode_frame and FrameDecoder (pipe.md, «UDP: the frame»).
// Golden frame bytes are literals; the ones built by `frame()` from the helpers do not depend on the code under test.
use super::*;
use crate::support::{frame, pattern};

mod encode_frame_of {
    use super::*;

    #[test]
    fn empty_datagram_is_four_zero_bytes() {
        assert_eq!(encode_frame(b""), [0, 0, 0, 0]);
    }

    #[test]
    fn three_bytes_get_length_three_before_them() {
        assert_eq!(encode_frame(b"abc"), [0, 0, 0, 3, 0x61, 0x62, 0x63]);
    }

    #[test]
    fn body_bytes_are_copied_verbatim_including_zero_and_ff() {
        let body: Vec<u8> = (0..=255).collect();
        let encoded = encode_frame(&body);
        assert_eq!(encoded[..4], [0, 0, 1, 0], "length 256 — big-endian");
        assert_eq!(encoded[4..], body[..]);
    }

    #[test]
    fn length_65535_fills_two_low_bytes() {
        let encoded = encode_frame(&vec![7u8; 65_535]);
        assert_eq!(encoded[..4], [0, 0, 0xFF, 0xFF]);
        assert_eq!(encoded.len(), 4 + 65_535);
    }

    #[test]
    fn length_above_65535_is_not_capped() {
        let encoded = encode_frame(&vec![7u8; 70_000]);
        assert_eq!(encoded[..4], [0x00, 0x01, 0x11, 0x70], "70000 = 0x00011170");
        assert_eq!(encoded.len(), 4 + 70_000);
    }
}

mod frame_decoder {
    use super::*;

    mod one_chunk {
        use super::*;

        #[test]
        fn single_frame_gives_its_body() {
            let mut decoder = FrameDecoder::default();
            assert_eq!(decoder.push(&[0, 0, 0, 3, 1, 2, 3]), vec![vec![1, 2, 3]]);
        }

        #[test]
        fn two_frames_give_both_bodies_in_order() {
            let mut decoder = FrameDecoder::default();
            let bodies = decoder.push(&[0, 0, 0, 2, 0xAA, 0xBB, 0, 0, 0, 1, 0xCC]);
            assert_eq!(bodies, vec![vec![0xAA, 0xBB], vec![0xCC]]);
        }

        #[test]
        fn zero_length_frame_gives_empty_body() {
            let mut decoder = FrameDecoder::default();
            assert_eq!(decoder.push(&[0, 0, 0, 0]), vec![Vec::<u8>::new()]);
        }

        #[test]
        fn zero_length_frames_between_others_keep_their_place() {
            let mut decoder = FrameDecoder::default();
            let bodies = decoder.push(&[0, 0, 0, 0, 0, 0, 0, 1, 0x41, 0, 0, 0, 0]);
            assert_eq!(bodies, vec![vec![], vec![0x41], vec![]]);
        }

        #[test]
        fn exact_frames_leave_nothing_buffered() {
            let mut decoder = FrameDecoder::default();
            decoder.push(&[0, 0, 0, 1, 9, 0, 0, 0, 1, 8]);
            assert!(decoder.buf.is_empty(), "remainder: {:?}", decoder.buf);
        }
    }

    mod split_stream {
        use super::*;

        #[test]
        fn byte_by_byte_frame_appears_only_on_the_last_byte() {
            let mut decoder = FrameDecoder::default();
            let stream = [0u8, 0, 0, 3, 7, 8, 9];
            for byte in &stream[..6] {
                assert!(
                    decoder.push(&[*byte]).is_empty(),
                    "frame came out before the last byte"
                );
            }
            assert_eq!(decoder.push(&stream[6..]), vec![vec![7, 8, 9]]);
        }

        #[test]
        fn header_cut_after_three_bytes_waits_for_the_fourth() {
            let mut decoder = FrameDecoder::default();
            assert!(decoder.push(&[0, 0, 0]).is_empty());
            assert_eq!(decoder.push(&[2, 5, 6]), vec![vec![5, 6]]);
        }

        #[test]
        fn body_cut_in_the_middle_waits_for_the_rest() {
            let mut decoder = FrameDecoder::default();
            assert!(decoder.push(&[0, 0, 0, 4, 1, 2]).is_empty());
            assert_eq!(decoder.push(&[3, 4]), vec![vec![1, 2, 3, 4]]);
        }

        #[test]
        fn tail_of_the_next_frame_is_kept_after_the_first() {
            let mut decoder = FrameDecoder::default();
            assert_eq!(decoder.push(&[0, 0, 0, 1, 0xAA, 0, 0]), vec![vec![0xAA]]);
            assert_eq!(decoder.push(&[0, 2, 0xBB, 0xCC]), vec![vec![0xBB, 0xCC]]);
        }

        #[test]
        fn frame_of_65535_bytes_assembles_from_1000_byte_chunks() {
            let body = pattern(65_535, 11);
            let stream = frame(&body);
            let mut decoder = FrameDecoder::default();
            let bodies: Vec<Vec<u8>> = stream.chunks(1000).flat_map(|c| decoder.push(c)).collect();
            assert_eq!(bodies, vec![body]);
        }

        #[test]
        fn hundred_frames_decode_the_same_at_any_chunk_size() {
            let bodies: Vec<Vec<u8>> = (0..100)
                .map(|i| pattern(i * 13 % 200, i as u32 + 1))
                .collect();
            let stream: Vec<u8> = bodies.iter().flat_map(|b| frame(b)).collect();
            for chunk_size in [1, 3, 7, 4096] {
                let mut decoder = FrameDecoder::default();
                let decoded: Vec<Vec<u8>> = stream
                    .chunks(chunk_size)
                    .flat_map(|c| decoder.push(c))
                    .collect();
                assert_eq!(decoded, bodies, "chunks of {chunk_size} bytes");
            }
        }
    }

    mod broken_stream {
        use super::*;

        #[test]
        fn cut_mid_frame_gives_nothing_and_keeps_the_bytes() {
            // JS: a stream cut in the middle of a frame gives neither a frame nor an error — the tail stays in the buffer.
            let mut decoder = FrameDecoder::default();
            assert!(decoder.push(&[0, 0, 0, 10, 1, 2, 3]).is_empty());
            assert_eq!(
                decoder.buf.len(),
                7,
                "header and three body bytes await the rest"
            );
            let rest = decoder.push(&[4, 5, 6, 7, 8, 9, 10]);
            assert_eq!(rest, vec![vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]]);
        }

        #[test]
        fn length_is_unsigned_so_a_huge_length_just_waits() {
            let mut decoder = FrameDecoder::default();
            assert!(decoder.push(&[0x80, 0, 0, 0, 1, 2, 3]).is_empty());
            assert_eq!(decoder.buf.len(), 7);
        }
    }

    mod round_trip {
        use super::*;

        #[test]
        fn encoded_datagrams_decode_to_themselves() {
            for len in [0, 1, 2047, 2048, 2049, 65_535] {
                let body = pattern(len, 5);
                let mut decoder = FrameDecoder::default();
                assert_eq!(
                    decoder.push(&encode_frame(&body)),
                    vec![body],
                    "length {len}"
                );
            }
        }

        #[test]
        fn encode_frame_matches_the_independent_reference() {
            for len in [0, 1, 255, 256, 4000] {
                let body = pattern(len, 9);
                assert_eq!(encode_frame(&body), frame(&body), "length {len}");
            }
        }
    }
}
