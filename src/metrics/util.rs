use num_traits::ToPrimitive;
use prometheus::core::Desc;
use prometheus::proto::{LabelPair, MetricFamily, MetricType};

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
