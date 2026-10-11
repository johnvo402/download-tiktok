//! Source-clock animation and deliberate transition overlaps, shared by persistence/export.
use crate::timeline::{Clip, ClipContent, TimelineDocument, MAX_TIME_US};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Keyframe {
    pub source_us: u64,
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub opacity: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualEffects {
    pub fade_in_us: u64,
    pub fade_out_us: u64,
    pub keyframes: Vec<Keyframe>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TransitionKind {
    Dissolve,
    WipeLeft,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transition {
    pub kind: TransitionKind,
    pub from_clip_id: String,
    pub duration_us: u64,
}
pub fn validate_effects(c: &Clip) -> Result<(), String> {
    let Some(e) = &c.effects else {
        return Ok(());
    };
    if matches!(
        c.content,
        ClipContent::Audio { .. } | ClipContent::Voice { .. } | ClipContent::SourceAudio { .. }
    ) || e.keyframes.len() > 64
        || e.fade_in_us > MAX_TIME_US
        || e.fade_out_us > MAX_TIME_US
    {
        return Err("Fade hình/keyframe không hợp lệ (tối đa 64 keyframe).".into());
    }
    for (i, k) in e.keyframes.iter().enumerate() {
        if k.source_us > MAX_TIME_US
            || i > 0 && k.source_us <= e.keyframes[i - 1].source_us
            || ![k.x, k.y, k.scale_x, k.scale_y, k.opacity]
                .iter()
                .all(|v| v.is_finite())
            || k.scale_x <= 0.0
            || k.scale_y <= 0.0
            || k.scale_x > 100.0
            || k.scale_y > 100.0
            || !(0.0..=1.0).contains(&k.opacity)
        {
            return Err(
                "Keyframe phải tăng dần theo thời gian nguồn; scale và opacity phải hợp lệ.".into(),
            );
        }
    }
    Ok(())
}
pub fn validate_transitions(doc: &TimelineDocument) -> Result<(), String> {
    for c in &doc.clips {
        let Some(t) = &c.transition_in else {
            continue;
        };
        let a = doc
            .clips
            .iter()
            .find(|a| a.id == t.from_clip_id)
            .ok_or("Nguồn chuyển cảnh không tồn tại.")?;
        let visual = |c: &Clip| {
            matches!(
                c.content,
                ClipContent::Video { .. } | ClipContent::Image { .. }
            )
        };
        if a.track_id != c.track_id
            || !visual(a)
            || !visual(c)
            || t.duration_us == 0
            || t.duration_us >= a.duration_us()?.min(c.duration_us()?)
            || a.start_us >= c.start_us
            || Some(a.end_us()?) != c.start_us.checked_add(t.duration_us)
            || doc.clips.iter().any(|b| {
                b.track_id == c.track_id && b.start_us > a.start_us && b.start_us < c.start_us
            })
        {
            return Err("Chuyển cảnh cần hai clip hình liền kề, chồng đúng thời lượng và còn phần nguồn riêng. Gỡ chuyển cảnh trước khi đổi điểm cắt/tốc độ.".into());
        }
    }
    Ok(())
}
pub fn owner<'a>(c: &'a Clip, doc: &'a TimelineDocument) -> &'a Clip {
    c.linked_clip_id
        .as_ref()
        .and_then(|id| doc.clips.iter().find(|p| p.id == *id))
        .unwrap_or(c)
}
pub fn transition_overlap(a: &Clip, b: &Clip, doc: &TimelineDocument) -> Result<bool, String> {
    let (mut left, mut right) = (owner(a, doc), owner(b, doc));
    if left
        .transition_in
        .as_ref()
        .is_some_and(|t| t.from_clip_id == right.id)
    {
        (left, right) = (right, left);
    }
    Ok(right
        .transition_in
        .as_ref()
        .is_some_and(|t| t.from_clip_id == left.id)
        && a.start_us.max(b.start_us) >= right.start_us
        && a.end_us()?.min(b.end_us()?)
            <= right.start_us + right.transition_in.as_ref().map_or(0, |t| t.duration_us))
}
/// Returns incoming/outgoing audio ramps for a clip or a linked layer.
pub fn audio_ramps(
    c: &Clip,
    doc: &TimelineDocument,
) -> Result<(Option<(u64, u64)>, Option<(u64, u64)>), String> {
    let c = owner(c, doc);
    let incoming = c
        .transition_in
        .as_ref()
        .map(|t| (c.start_us, t.duration_us));
    let outgoing = doc
        .clips
        .iter()
        .find(|n| {
            n.transition_in
                .as_ref()
                .is_some_and(|t| t.from_clip_id == c.id)
        })
        .map(|n| {
            (
                c.end_us().unwrap_or(0),
                n.transition_in.as_ref().unwrap().duration_us,
            )
        });
    Ok((incoming, outgoing))
}
pub fn audio_gain(ramps: &(Option<(u64, u64)>, Option<(u64, u64)>), time: f64) -> f64 {
    ramps
        .0
        .map_or(1.0, |(a, d)| ((time - a as f64) / d as f64).clamp(0.0, 1.0))
        * ramps
            .1
            .map_or(1.0, |(b, d)| ((b as f64 - time) / d as f64).clamp(0.0, 1.0))
}
pub fn motion_at(c: &Clip, source: u64) -> [f64; 5] {
    let values = |k: &Keyframe| [k.x, k.y, k.scale_x, k.scale_y, k.opacity];
    let Some(keys) = c
        .effects
        .as_ref()
        .map(|e| &e.keyframes)
        .filter(|k| !k.is_empty())
    else {
        let t = &c.transform;
        return [t.x, t.y, t.scale_x, t.scale_y, t.opacity];
    };
    let a = keys
        .iter()
        .rev()
        .find(|k| k.source_us <= source)
        .unwrap_or(&keys[0]);
    let b = keys
        .iter()
        .find(|k| k.source_us >= source)
        .unwrap_or(keys.last().unwrap());
    let p = if a.source_us == b.source_us {
        0.0
    } else {
        (source as f64 - a.source_us as f64) / (b.source_us - a.source_us) as f64
    };
    let (a, b) = (values(a), values(b));
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * p)
}
fn num(v: f64) -> String {
    format!("{v:.9}")
}
/// FFmpeg expression; all values are validated numbers, never user strings.
pub fn motion_expression(c: &Clip, index: usize, local_seconds: &str) -> String {
    let fallback = motion_at(c, 0)[index];
    let Some(keys) = c
        .effects
        .as_ref()
        .map(|e| &e.keyframes)
        .filter(|k| !k.is_empty())
    else {
        return num(fallback);
    };
    let get = |k: &Keyframe| [k.x, k.y, k.scale_x, k.scale_y, k.opacity][index];
    let source = format!(
        "min({},{}+floor(({local_seconds})*{}*1000000+0.5))",
        c.source_out_us - 1,
        c.source_in_us,
        num(c.rate.numerator as f64 / c.rate.denominator as f64)
    );
    let mut expression = num(get(keys.last().unwrap()));
    for pair in keys.windows(2).rev() {
        let (a, b) = (&pair[0], &pair[1]);
        let value = format!(
            "{}+({}-{})*max(0,min(1,(({source})-{})/{}))",
            num(get(a)),
            num(get(b)),
            num(get(a)),
            a.source_us,
            b.source_us - a.source_us
        );
        expression = format!("if(lt({source},{}),{value},{expression})", b.source_us);
    }
    expression
}
pub fn alpha_expression(
    c: &Clip,
    doc: &TimelineDocument,
    local_seconds: &str,
) -> Result<String, String> {
    let mut expression = motion_expression(c, 4, local_seconds);
    if let Some(e) = &c.effects {
        let d = c.duration_us()?;
        if e.fade_in_us > 0 {
            expression.push_str(&format!(
                "*max(0,min(1,({local_seconds})/{}))",
                num(e.fade_in_us.min(d) as f64 / 1e6)
            ));
        }
        if e.fade_out_us > 0 {
            expression.push_str(&format!(
                "*max(0,min(1,({}-({local_seconds}))/{}))",
                num(d as f64 / 1e6),
                num(e.fade_out_us.min(d) as f64 / 1e6)
            ));
        }
    }
    if c.linked_clip_id.is_some() {
        let ramps = audio_ramps(c, doc)?;
        let global = format!("({local_seconds})+{}", num(c.start_us as f64 / 1e6));
        if let Some((a, d)) = ramps.0 {
            expression.push_str(&format!(
                "*max(0,min(1,(({global})-{})/{}))",
                num(a as f64 / 1e6),
                num(d as f64 / 1e6)
            ));
        }
        if let Some((b, d)) = ramps.1 {
            expression.push_str(&format!(
                "*max(0,min(1,({}-({global}))/{}))",
                num(b as f64 / 1e6),
                num(d as f64 / 1e6)
            ));
        }
    }
    if let Some(t) = &c.transition_in {
        let progress = format!(
            "max(0,min(1,({local_seconds})/{}))",
            num(t.duration_us as f64 / 1e6)
        );
        expression.push_str(&match t.kind {
            TransitionKind::Dissolve => format!("*({progress})"),
            TransitionKind::WipeLeft => format!("*lt(X/W,{progress})"),
        });
    }
    Ok(expression)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        timeline::{self, Rational},
        timeline_edit::{apply_layout, EditLayout},
    };

    #[test]
    fn shared_motion_clock_and_fade_samples() {
        let (_dir, _store, p, _, _) = crate::video_content::tests::fixture();
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../../test-fixtures/timeline-effects.json"))
                .unwrap();
        let mut c = p.timeline.unwrap().clips.remove(0);
        c.start_us = data["startUs"].as_u64().unwrap();
        c.source_in_us = data["sourceInUs"].as_u64().unwrap();
        c.source_out_us = data["sourceOutUs"].as_u64().unwrap();
        c.rate = serde_json::from_value(data["rate"].clone()).unwrap();
        c.effects = Some(serde_json::from_value(data["effects"].clone()).unwrap());
        for sample in data["samples"].as_array().unwrap() {
            let at = sample["timeUs"].as_u64().unwrap();
            let values = motion_at(&c, c.timeline_to_source_us(at).unwrap().unwrap());
            for (i, v) in values.iter().enumerate() {
                assert!((*v - sample["motion"][i].as_f64().unwrap()).abs() < 1e-9);
            }
            let e = c.effects.as_ref().unwrap();
            let fade = ((at - c.start_us) as f64 / e.fade_in_us as f64)
                .min((c.end_us().unwrap() - at) as f64 / e.fade_out_us as f64)
                .min(1.0);
            assert!((values[4] * fade - sample["opacity"].as_f64().unwrap()).abs() < 1e-9);
        }
    }
    #[test]
    fn effects_transitions_persist_undo_and_reject_invalid_edits_atomically() {
        let (_dir, store, p, _, _) = crate::video_content::tests::fixture();
        let before = EditLayout::from_document(p.timeline.as_ref().unwrap());
        let mut next = before.clone();
        let a = next.clips[0].id.clone();
        next.clips[0].start_us = 0;
        next.clips[0].source_in_us = 0;
        next.clips[0].source_out_us = 2_000_000;
        next.clips[0].rate = Rational::default();
        next.clips[1].track_id = next.clips[0].track_id.clone();
        next.clips[1].start_us = 1_000_000;
        next.clips[1].source_in_us = 0;
        next.clips[1].source_out_us = 2_000_000;
        next.clips[1].transition_in = Some(Transition {
            kind: TransitionKind::Dissolve,
            from_clip_id: a,
            duration_us: 1_000_000,
        });
        next.clips[0].effects = Some(VisualEffects {
            fade_in_us: 100_000,
            fade_out_us: 200_000,
            keyframes: vec![
                Keyframe {
                    source_us: 0,
                    x: 0.5,
                    y: 0.5,
                    scale_x: 1.0,
                    scale_y: 1.0,
                    opacity: 0.5,
                },
                Keyframe {
                    source_us: 2_000_000,
                    x: 0.75,
                    y: 0.5,
                    scale_x: 0.5,
                    scale_y: 0.5,
                    opacity: 1.0,
                },
            ],
        });
        let saved = apply_layout(&store, &p.id, before.clone(), next.clone()).unwrap();
        assert_eq!(
            saved.timeline.as_ref().unwrap().video_contents,
            p.timeline.as_ref().unwrap().video_contents
        );
        assert_eq!(
            store.get(&p.id).unwrap().timeline.unwrap().clips,
            next.clips
        );
        let mut bad = next.clone();
        bad.clips[1].transition_in.as_mut().unwrap().from_clip_id = bad.clips[1].id.clone();
        assert!(apply_layout(&store, &p.id, next.clone(), bad).is_err());
        let mut bad = next.clone();
        bad.clips[0].effects.as_mut().unwrap().keyframes[1].source_us = 0;
        assert!(apply_layout(&store, &p.id, next.clone(), bad).is_err());
        let mut bad = next.clone();
        bad.clips[1].transition_in = None;
        assert!(apply_layout(&store, &p.id, next.clone(), bad).is_err());
        assert_eq!(
            store.get(&p.id).unwrap().timeline.unwrap().clips,
            next.clips
        );
        apply_layout(&store, &p.id, next.clone(), before.clone()).unwrap();
        let saved = apply_layout(&store, &p.id, before, next).unwrap();
        timeline::validate(saved.timeline.as_ref().unwrap()).unwrap();
    }
    #[test]
    fn linked_cues_may_overlap_only_within_the_transition_and_old_clips_remain_readable() {
        let (_dir, _store, p, _, _) = crate::video_content::tests::fixture();
        let mut doc = p.timeline.unwrap();
        let old = serde_json::to_value(&doc.clips[0]).unwrap();
        assert!(old.get("effects").is_none() && old.get("transitionIn").is_none());
        let _: Clip = serde_json::from_value(old).unwrap();
        doc.clips[0].source_out_us = 2_000_000;
        doc.clips[1].source_out_us = 2_000_000;
        doc.clips[1].start_us = 1_000_000;
        doc.clips[1].track_id = doc.clips[0].track_id.clone();
        doc.clips[1].transition_in = Some(Transition {
            kind: TransitionKind::WipeLeft,
            from_clip_id: doc.clips[0].id.clone(),
            duration_us: 1_000_000,
        });
        let mut a = doc.clips[0].clone();
        a.linked_clip_id = Some(a.id.clone());
        a.id = uuid::Uuid::new_v4().to_string();
        a.transition_in = None;
        let mut b = doc.clips[1].clone();
        b.linked_clip_id = Some(b.id.clone());
        b.id = uuid::Uuid::new_v4().to_string();
        b.transition_in = None;
        a.start_us = 1_500_000;
        a.source_in_us = 1_500_000;
        b.start_us = 1_250_000;
        b.source_in_us = 250_000;
        assert!(transition_overlap(&b, &a, &doc).unwrap());
        let ramps = audio_ramps(&a, &doc).unwrap();
        assert!((audio_gain(&ramps, 1_500_000.0) - 0.5).abs() < 1e-9);
        a.source_out_us = 3_000_000;
        assert!(!transition_overlap(&a, &b, &doc).unwrap());
    }
}
