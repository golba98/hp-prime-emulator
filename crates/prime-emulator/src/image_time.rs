//! Process-local timing fields encoded relative to one image transaction.
//! Restoring rebases the complete graph once, preserving ordering, elapsed
//! durations and future deadlines while excluding time outside the session.
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use std::{
    cell::Cell,
    collections::VecDeque,
    time::{Duration, Instant},
};
thread_local! { static ANCHOR: Cell<Option<Instant>> = const { Cell::new(None) }; }
struct Scope(Option<Instant>);
impl Drop for Scope {
    fn drop(&mut self) {
        ANCHOR.with(|anchor| anchor.set(self.0));
    }
}
pub(crate) fn at<T>(anchor: Instant, operation: impl FnOnce() -> T) -> T {
    let _scope = Scope(ANCHOR.with(|old| old.replace(Some(anchor))));
    operation()
}
fn anchor() -> Result<Instant, String> {
    ANCHOR
        .with(Cell::get)
        .ok_or_else(|| "Timing image requires an explicit transaction origin".into())
}
pub(crate) trait Portable: Sized {
    type Image: Serialize + DeserializeOwned;
    fn image(&self) -> Result<Self::Image, String>;
    fn restore(image: Self::Image) -> Result<Self, String>;
}
impl Portable for Instant {
    type Image = i64;
    fn image(&self) -> Result<i64, String> {
        let origin = anchor()?;
        let (negative, elapsed) = if *self < origin {
            (true, origin.duration_since(*self))
        } else {
            (false, self.duration_since(origin))
        };
        let nanos = i64::try_from(elapsed.as_nanos())
            .map_err(|_| "Timing image exceeds signed nanosecond range")?;
        Ok(if negative { -nanos } else { nanos })
    }
    fn restore(delta: i64) -> Result<Self, String> {
        let origin = anchor()?;
        let elapsed = Duration::from_nanos(delta.unsigned_abs());
        (if delta < 0 {
            origin.checked_sub(elapsed)
        } else {
            origin.checked_add(elapsed)
        })
        .ok_or_else(|| "Timing image cannot be represented by this monotonic clock".into())
    }
}
impl<T: Portable> Portable for Option<T> {
    type Image = Option<T::Image>;
    fn image(&self) -> Result<Self::Image, String> {
        self.as_ref().map(Portable::image).transpose()
    }
    fn restore(image: Self::Image) -> Result<Self, String> {
        image.map(T::restore).transpose()
    }
}
impl<A: Clone + Serialize + DeserializeOwned> Portable for (A, Instant) {
    type Image = (A, i64);
    fn image(&self) -> Result<Self::Image, String> {
        Ok((self.0.clone(), self.1.image()?))
    }
    fn restore(image: Self::Image) -> Result<Self, String> {
        Ok((image.0, Instant::restore(image.1)?))
    }
}
impl<A: Clone + Serialize + DeserializeOwned> Portable for (A, Option<Instant>) {
    type Image = (A, Option<i64>);
    fn image(&self) -> Result<Self::Image, String> {
        Ok((self.0.clone(), self.1.image()?))
    }
    fn restore(image: Self::Image) -> Result<Self, String> {
        Ok((image.0, Option::<Instant>::restore(image.1)?))
    }
}
impl<A: Clone + Serialize + DeserializeOwned, B: Clone + Serialize + DeserializeOwned> Portable
    for (A, B, Instant)
{
    type Image = (A, B, i64);
    fn image(&self) -> Result<Self::Image, String> {
        Ok((self.0.clone(), self.1.clone(), self.2.image()?))
    }
    fn restore(image: Self::Image) -> Result<Self, String> {
        Ok((image.0, image.1, Instant::restore(image.2)?))
    }
}
impl Portable for (Instant, bool) {
    type Image = (i64, bool);
    fn image(&self) -> Result<Self::Image, String> {
        Ok((self.0.image()?, self.1))
    }
    fn restore(image: Self::Image) -> Result<Self, String> {
        Ok((Instant::restore(image.0)?, image.1))
    }
}
impl Portable for VecDeque<(Instant, bool)> {
    type Image = VecDeque<(i64, bool)>;
    fn image(&self) -> Result<Self::Image, String> {
        self.iter().map(|(t, v)| Ok((t.image()?, *v))).collect()
    }
    fn restore(image: Self::Image) -> Result<Self, String> {
        image
            .into_iter()
            .map(|(t, v)| Ok((Instant::restore(t)?, v)))
            .collect()
    }
}
pub(crate) fn serialize<T: Portable, S: Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value
        .image()
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}
pub(crate) fn deserialize<'de, T: Portable, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    T::restore(T::Image::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Serialize, Deserialize)]
    struct Timeline {
        #[serde(with = "super")]
        start: Instant,
        #[serde(with = "super")]
        observations: VecDeque<(Instant, bool)>,
        #[serde(with = "super")]
        due: Option<(u16, Instant)>,
    }
    #[test]
    fn one_transaction_preserves_order_and_exact_remaining_time_across_rebase() {
        let at = Instant::now();
        let timeline = Timeline {
            start: at - Duration::from_millis(50),
            observations: [(at, true), (at + Duration::from_millis(5), false)].into(),
            due: Some((12, at + Duration::from_millis(450))),
        };
        let json = super::at(at, || serde_json::to_vec(&timeline)).unwrap();
        assert!(serde_json::to_vec(&timeline).is_err());
        let later = at + Duration::from_secs(90);
        let restored: Timeline = super::at(later, || serde_json::from_slice(&json)).unwrap();
        assert_eq!(restored.start, later - Duration::from_millis(50));
        assert_eq!(restored.observations[0].0, later);
        assert_eq!(
            restored.observations[1].0 - restored.observations[0].0,
            Duration::from_millis(5)
        );
        assert_eq!(restored.due, Some((12, later + Duration::from_millis(450))));
        assert!(anchor().is_err());
    }
}
