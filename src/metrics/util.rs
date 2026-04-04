use num_traits::ToPrimitive;
use prometheus::core::Desc;
use prometheus::proto::{LabelPair, MetricFamily, MetricType};
use std::sync::{Arc, Mutex};

pub struct Measurement<T> {
    inner: Arc<Mutex<Option<T>>>,
}

impl<T> Clone for Measurement<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Measurement<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub fn update_if(&self, value: Option<T>, predicate: impl Fn(&T, &T) -> bool) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        // If a metric collector has failed, we want to
        // stop exposing that metric instead of reporting
        // the previous, stale value
        let Some(value) = value else {
            *guard = None;
            return;
        };

        match guard.as_ref() {
            None => *guard = Some(value),
            Some(prev) => {
                if predicate(prev, &value) {
                    *guard = Some(value);
                }
            }
        }
    }

    pub fn read<R>(&self, mapper: impl FnOnce(&T) -> R) -> Option<R> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(mapper)
    }
}

pub fn into_labels(kv: &[(&str, &str)]) -> Vec<LabelPair> {
    kv.iter()
        .copied()
        .map(|(k, v)| {
            let mut lp = LabelPair::default();
            lp.set_name(k.to_owned());
            lp.set_value(v.to_owned());
            lp
        })
        .collect()
}

pub fn maybe_gauge<T>(
    families: &mut Vec<MetricFamily>,
    desc: &Desc,
    labels: &[LabelPair],
    val: Option<T>,
) where
    T: ToPrimitive,
{
    if let Some(v) = val {
        families.push(gauge(desc, labels.to_vec(), v.to_f64().unwrap_or(f64::NAN)));
    }
}

pub fn maybe_counter<T>(
    families: &mut Vec<MetricFamily>,
    desc: &Desc,
    labels: &[LabelPair],
    val: Option<T>,
) where
    T: ToPrimitive,
{
    if let Some(v) = val {
        families.push(counter(
            desc,
            labels.to_vec(),
            v.to_f64().unwrap_or(f64::NAN),
        ));
    }
}

pub fn gauge(desc: &Desc, label_values: Vec<LabelPair>, value: f64) -> MetricFamily {
    let mut mf = MetricFamily::default();
    mf.set_name(desc.fq_name.clone());
    mf.set_help(desc.help.clone());
    mf.set_field_type(MetricType::GAUGE);

    let mut m = prometheus::proto::Metric::default();
    m.set_label(label_values);

    let mut g = prometheus::proto::Gauge::default();
    g.set_value(value);
    m.set_gauge(g);

    mf.set_metric(vec![m]);
    mf
}

pub fn counter(desc: &Desc, label_values: Vec<LabelPair>, value: f64) -> MetricFamily {
    let mut mf = MetricFamily::default();
    mf.set_name(desc.fq_name.clone());
    mf.set_help(desc.help.clone());
    mf.set_field_type(MetricType::COUNTER);

    let mut m = prometheus::proto::Metric::default();
    m.set_label(label_values);

    let mut c = prometheus::proto::Counter::default();
    c.set_value(value);
    m.set_counter(c);

    mf.set_metric(vec![m]);
    mf
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_desc(name: &str) -> Desc {
        Desc::new(name.into(), "test".into(), vec![], HashMap::new()).unwrap()
    }

    #[test]
    fn test_maybe_gauge_none_adds_nothing() {
        let mut families = Vec::new();
        let desc = create_test_desc("test_gauge");
        maybe_gauge::<f64>(&mut families, &desc, &[], None);
        assert!(families.is_empty());
    }

    #[test]
    fn test_maybe_gauge_some_adds_gauge() {
        let mut families = Vec::new();
        let desc = create_test_desc("test_gauge");
        maybe_gauge(&mut families, &desc, &[], Some(42.0));
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].get_field_type(), MetricType::GAUGE);
    }

    #[test]
    fn test_maybe_counter_none_adds_nothing() {
        let mut families = Vec::new();
        let desc = create_test_desc("test_counter");
        maybe_counter::<u64>(&mut families, &desc, &[], None);
        assert!(families.is_empty());
    }

    #[test]
    fn test_maybe_counter_some_adds_counter() {
        let mut families = Vec::new();
        let desc = create_test_desc("test_counter");
        maybe_counter(&mut families, &desc, &[], Some(100u64));
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].get_field_type(), MetricType::COUNTER);
    }

    #[test]
    fn test_into_labels() {
        let labels = into_labels(&[("device", "sda"), ("model", "WD")]);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].name(), "device");
        assert_eq!(labels[0].value(), "sda");
        assert_eq!(labels[1].name(), "model");
        assert_eq!(labels[1].value(), "WD");
    }

    #[test]
    fn test_store_new_is_empty() {
        let store = Measurement::<i32>::new();
        assert!(store.read(|v| *v).is_none());
    }

    #[test]
    fn test_store_update_sets_value() {
        let store = Measurement::new();
        store.update_if(Some(42), |_, _| true);
        assert_eq!(store.read(|v| *v), Some(42));
    }

    #[test]
    fn test_store_update_none_clears() {
        let store = Measurement::new();
        store.update_if(Some(42), |_, _| true);
        store.update_if(None, |_, _| true);
        assert!(store.read(|v| *v).is_none());
    }

    #[test]
    fn test_store_predicate_respected() {
        let store = Measurement::new();
        store.update_if(Some(10), |_, _| true);
        store.update_if(Some(5), |old, new| *new > *old);
        assert_eq!(store.read(|v| *v), Some(10));
    }

    #[test]
    fn test_store_clone_shares_state() {
        let store = Measurement::new();
        let clone = store.clone();
        store.update_if(Some(99), |_, _| true);
        assert_eq!(clone.read(|v| *v), Some(99));
    }
}
