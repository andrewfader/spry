//! Animation timing: which frame shows at a given moment.
//!
//! Playback is a pure function of time, so the preview, the GIF exporter
//! and the tests all agree on it. Ask "what shows at 250 ms?" and the answer
//! is the same every time, with no clock and no per-tick state.

use super::sprite::{LoopDirection, Sprite, Tag};

/// The frames of one loop through `tag` (or the whole sprite) in play
/// order. Ping-pong doesn't repeat its end frames, so `1,2,3` plays as
/// `1,2,3,2` and then loops.
pub fn play_order(sprite: &Sprite, tag: Option<&Tag>) -> Vec<usize> {
    let last = sprite.frame_count().saturating_sub(1);
    let (from, to, direction) = match tag {
        Some(tag) => (tag.from.min(last), tag.to.min(last), tag.direction),
        None => (0, last, LoopDirection::Forward),
    };
    let (from, to) = (from.min(to), from.max(to));
    let forward: Vec<usize> = (from..=to).collect();
    match direction {
        LoopDirection::Forward => forward,
        LoopDirection::Reverse => forward.into_iter().rev().collect(),
        LoopDirection::PingPong => {
            let mut order = forward.clone();
            if forward.len() > 2 {
                order.extend(forward[1..forward.len() - 1].iter().rev());
            } else if forward.len() == 2 {
                order = forward;
            }
            order
        }
    }
}

pub fn loop_duration_ms(sprite: &Sprite, tag: Option<&Tag>) -> u64 {
    play_order(sprite, tag).iter().map(|&f| sprite.frames[f].duration_ms as u64).sum()
}

/// The frame showing `elapsed_ms` after playback started, looping forever.
pub fn frame_at(sprite: &Sprite, tag: Option<&Tag>, elapsed_ms: u64) -> usize {
    let order = play_order(sprite, tag);
    let total = loop_duration_ms(sprite, tag).max(1);
    let mut remaining = elapsed_ms % total;
    for &frame in &order {
        let duration = sprite.frames[frame].duration_ms as u64;
        if remaining < duration {
            return frame;
        }
        remaining -= duration;
    }
    order.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite_with_durations(durations: &[u32]) -> Sprite {
        let mut sprite = Sprite::new(1, 1);
        sprite.frames[0].duration_ms = durations[0];
        for (i, &d) in durations.iter().enumerate().skip(1) {
            sprite.insert_frame(i, d);
        }
        sprite
    }

    fn tag(from: usize, to: usize, direction: LoopDirection) -> Tag {
        Tag { name: "t".into(), from, to, direction }
    }

    #[test]
    fn playback_honours_each_frames_duration() {
        let sprite = sprite_with_durations(&[100, 200, 100]);
        assert_eq!(frame_at(&sprite, None, 0), 0);
        assert_eq!(frame_at(&sprite, None, 99), 0);
        assert_eq!(frame_at(&sprite, None, 250), 1);
        assert_eq!(frame_at(&sprite, None, 300), 2);
        assert_eq!(frame_at(&sprite, None, 400), 0, "loops after 400 ms");
    }

    #[test]
    fn ping_pong_bounces_without_repeating_the_ends() {
        let sprite = sprite_with_durations(&[100, 100, 100, 100]);
        assert_eq!(play_order(&sprite, Some(&tag(0, 2, LoopDirection::PingPong))), vec![0, 1, 2, 1]);
        assert_eq!(play_order(&sprite, Some(&tag(0, 1, LoopDirection::PingPong))), vec![0, 1]);
    }

    #[test]
    fn reverse_tags_play_backwards() {
        let sprite = sprite_with_durations(&[100, 100, 100]);
        assert_eq!(play_order(&sprite, Some(&tag(0, 2, LoopDirection::Reverse))), vec![2, 1, 0]);
    }

    #[test]
    fn tags_past_the_last_frame_are_clamped() {
        let sprite = sprite_with_durations(&[100, 100]);
        assert_eq!(play_order(&sprite, Some(&tag(1, 9, LoopDirection::Forward))), vec![1]);
    }
}
