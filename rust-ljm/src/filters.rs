//! Noise filters applied to exported data (never to the archive).
//!
//! The same pipeline, with the same arithmetic, runs in the webapp's live plot
//! (`webapp/src/lib/filters.ts`); the shared vectors in `testdata/filter-vectors.json`
//! check that both give the same output. Per sample, in order:
//!
//! 1. Despike (raw volts): grey opening, a running minimum then a running maximum over
//!    `w = ceil(2.5 ms × fs)` samples. It removes upward spikes narrower than `w` and
//!    keeps wider pulses and anything downward. Output `j` needs input `j + w - 1`.
//! 2. and 3. Remove the 10 Hz and 11.906 Hz square waves (raw volts): subtract running
//!    templates of one period in phase bins, indexed by the sample count since the
//!    segment started (both sources run on the LabJack clock). Each bin keeps a
//!    running mean (cumulative at first, then exponential with a 30 s memory); the
//!    template is the bin mean minus the running mean of all samples. Templates are
//!    read before the sample updates them, and each learns from the sample minus the
//!    other's current template.
//! 4. Calibration.
//! 5. High-pass, then low-pass: 2nd-order Butterworth biquads. The live plot runs them
//!    causally; exports run them forward and backward ([`ExportFilter`]).
//!
//! The channel settings come from the export request (the webapp copies them from the
//! channel's `sensor_settings.filters` entry); see `docs/src/reference/kv-config.md`.

use std::collections::VecDeque;

use serde_json::Value;

/// Length of the despike window, ms.
pub const DESPIKE_WINDOW_MS: f64 = 2.5;
/// Frequency of the first square wave, Hz. Its period must be a whole number of samples.
pub const TEN_HZ: f64 = 10.0;
/// Frequency of the second square wave, Hz, measured on MU1 and MU2 (lines at 11.906,
/// 35.72 and 59.53 Hz). It also runs on the LabJack clock.
pub const ELEVEN_NINE_HZ: f64 = 11.906;
/// Memory of the template running means, seconds.
pub const TEMPLATE_MEMORY_S: f64 = 30.0;
/// Fewest samples per period for a template.
pub const MIN_TEMPLATE_PERIOD_SAMPLES: f64 = 4.0;
/// How far `fs / 10` may be from a whole number of samples.
pub const PERIOD_TOLERANCE_SAMPLES: f64 = 0.01;
/// Fewest phase bins of the fractional (11.9 Hz) template.
pub const MIN_FRACTIONAL_BINS: usize = 64;
/// Phase bins per sample of period in the fractional template.
pub const FRACTIONAL_BINS_PER_SAMPLE: usize = 4;
/// Highest usable cutoff, as a fraction of the sample rate.
pub const MAX_CUTOFF_FRACTION: f64 = 0.45;
/// Longest run of missing samples filtered through as `NaN`. A longer gap ends the
/// segment and every filter starts over.
pub const MAX_FILLED_GAP_SAMPLES: i64 = 4;

/// Filters of one channel, cleaned: `true` switches and positive cutoffs only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelFilterSettings {
    pub despike: bool,
    pub remove_10hz: bool,
    pub remove_11_9hz: bool,
    pub highpass_hz: Option<f64>,
    pub lowpass_hz: Option<f64>,
}

/// Reads a cutoff: a finite number above 0, or a string holding one.
fn cutoff(value: Option<&Value>) -> Option<f64> {
    let number = match value? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if !s.trim().is_empty() => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (number.is_finite() && number > 0.0).then_some(number)
}

impl ChannelFilterSettings {
    /// Cleans one channel's settings the way the webapp does: switches count only when
    /// they are exactly `true`, cutoffs only when finite and above 0; anything else
    /// (missing, `false`, `null`, malformed) is off.
    pub fn from_value(value: &Value) -> Self {
        let Some(object) = value.as_object() else {
            return Self::default();
        };
        let switch = |name: &str| object.get(name).and_then(Value::as_bool) == Some(true);
        Self {
            despike: switch("despike"),
            remove_10hz: switch("remove_10hz"),
            remove_11_9hz: switch("remove_11_9hz"),
            highpass_hz: cutoff(object.get("highpass_hz")),
            lowpass_hz: cutoff(object.get("lowpass_hz")),
        }
    }

    /// True when any filter is on.
    pub fn is_active(&self) -> bool {
        self.despike
            || self.remove_10hz
            || self.remove_11_9hz
            || self.highpass_hz.is_some()
            || self.lowpass_hz.is_some()
    }
}

/// What runs for one channel at one sample rate; from [`plan_filters`].
#[derive(Debug, Clone, PartialEq)]
pub struct FilterPlan {
    pub fs: f64,
    /// Despike window in samples, or 0 when off.
    pub despike_window: usize,
    /// 10 Hz period in samples, or 0 when off.
    pub period10: usize,
    /// 11.9 Hz period in samples (fractional), or 0 when off.
    pub period11: f64,
    /// Phase bins of the 11.9 Hz template.
    pub bins11: usize,
    /// High-pass cutoff, Hz, or 0 when off.
    pub highpass_hz: f64,
    /// Low-pass cutoff, Hz, or 0 when off.
    pub lowpass_hz: f64,
}

/// Works out which filters run at a sample rate; filters that cannot run (too low a
/// rate, a 10 Hz period that is not a whole number of samples, a cutoff at or above
/// 0.45 × the rate) are left off. Same rules as `planFilters` in the webapp.
pub fn plan_filters(settings: &ChannelFilterSettings, fs: f64) -> FilterPlan {
    let valid = fs.is_finite() && fs > 0.0;
    let mut plan = FilterPlan {
        fs,
        despike_window: 0,
        period10: 0,
        period11: 0.0,
        bins11: 0,
        highpass_hz: 0.0,
        lowpass_hz: 0.0,
    };
    if settings.despike && valid {
        let window = (fs * DESPIKE_WINDOW_MS / 1000.0 - 1e-9).ceil();
        if window >= 2.0 {
            plan.despike_window = window as usize;
        }
    }
    if settings.remove_10hz && valid {
        let exact = fs / TEN_HZ;
        let period = exact.round();
        if (exact - period).abs() <= PERIOD_TOLERANCE_SAMPLES
            && period >= MIN_TEMPLATE_PERIOD_SAMPLES
        {
            plan.period10 = period as usize;
        }
    }
    if settings.remove_11_9hz && valid {
        let period = fs / ELEVEN_NINE_HZ;
        if period >= MIN_TEMPLATE_PERIOD_SAMPLES {
            plan.period11 = period;
            plan.bins11 =
                MIN_FRACTIONAL_BINS.max(FRACTIONAL_BINS_PER_SAMPLE * period.ceil() as usize);
        }
    }
    if let Some(hz) = settings
        .highpass_hz
        .filter(|hz| valid && *hz < MAX_CUTOFF_FRACTION * fs)
    {
        plan.highpass_hz = hz;
    }
    if let Some(hz) = settings
        .lowpass_hz
        .filter(|hz| valid && *hz < MAX_CUTOFF_FRACTION * fs)
    {
        plan.lowpass_hz = hz;
    }
    plan
}

/// Normalised biquad coefficients (`a0 = 1`).
#[derive(Debug, Clone, Copy)]
struct BiquadCoefficients {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

/// 2nd-order Butterworth by the bilinear transform with prewarping (the audio-EQ
/// cookbook with Q = 1/√2).
fn butterworth(highpass: bool, cutoff_hz: f64, fs: f64) -> BiquadCoefficients {
    let w0 = 2.0 * std::f64::consts::PI * cutoff_hz / fs;
    let cos_w = w0.cos();
    let alpha = w0.sin() / std::f64::consts::SQRT_2;
    let a0 = 1.0 + alpha;
    let (b_mid, b_edge) = if highpass {
        (-(1.0 + cos_w), (1.0 + cos_w) / 2.0)
    } else {
        (1.0 - cos_w, (1.0 - cos_w) / 2.0)
    };
    BiquadCoefficients {
        b0: b_edge / a0,
        b1: b_mid / a0,
        b2: b_edge / a0,
        a1: -2.0 * cos_w / a0,
        a2: (1.0 - alpha) / a0,
    }
}

/// One biquad, transposed direct form II. The first finite sample after a reset sets
/// the steady state for that value; `NaN` in gives `NaN` out and keeps the state.
#[derive(Debug, Clone)]
struct Biquad {
    c: BiquadCoefficients,
    z1: f64,
    z2: f64,
    started: bool,
}

impl Biquad {
    fn new(c: BiquadCoefficients) -> Self {
        Self {
            c,
            z1: 0.0,
            z2: 0.0,
            started: false,
        }
    }

    #[cfg(test)]
    fn reset(&mut self) {
        self.started = false;
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    fn process(&mut self, x: f64) -> f64 {
        if !x.is_finite() {
            return f64::NAN;
        }
        let c = self.c;
        if !self.started {
            let gain = (c.b0 + c.b1 + c.b2) / (1.0 + c.a1 + c.a2);
            let y0 = gain * x;
            self.z2 = c.b2 * x - c.a2 * y0;
            self.z1 = c.b1 * x - c.a1 * y0 + self.z2;
            self.started = true;
        }
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

/// Streaming grey opening over `w` samples: output `j` is the largest of the minima of
/// the `w`-sample windows containing `j`, windows cut at the segment's ends and `NaN`
/// left out. Emitted once input `j + w - 1` has arrived, or by [`Self::flush`].
#[derive(Debug, Clone)]
struct Despiker {
    w: usize,
    xs: Vec<f64>,
    es: Vec<f64>,
    count: usize,
    emitted: usize,
}

impl Despiker {
    fn new(w: usize) -> Self {
        Self {
            w,
            xs: vec![0.0; 2 * w],
            es: vec![0.0; w],
            count: 0,
            emitted: 0,
        }
    }

    fn window_min(&self, from: i64, to: i64) -> f64 {
        let mut min = f64::NAN;
        let start = from.max(0);
        let end = to.min(self.count as i64 - 1);
        let mut m = start;
        while m <= end {
            let x = self.xs[m as usize % self.xs.len()];
            if !x.is_nan() && (min.is_nan() || x < min) {
                min = x;
            }
            m += 1;
        }
        min
    }

    fn slot(&self, k: i64) -> usize {
        k.rem_euclid(self.w as i64) as usize
    }

    fn emit(&mut self, j: usize, out: &mut Vec<f64>) {
        let w = self.w as i64;
        let j = j as i64;
        if j == 0 {
            for k in -(w - 1)..0 {
                let slot = self.slot(k);
                self.es[slot] = self.window_min(k, k + w - 1);
            }
        }
        let slot = self.slot(j);
        self.es[slot] = self.window_min(j, j + w - 1);
        let x = self.xs[j as usize % self.xs.len()];
        let mut y = f64::NAN;
        if !x.is_nan() {
            for k in (j - w + 1)..=j {
                let e = self.es[self.slot(k)];
                if !e.is_nan() && (y.is_nan() || e > y) {
                    y = e;
                }
            }
        }
        out.push(y);
        self.emitted = j as usize + 1;
    }

    fn push(&mut self, x: f64, out: &mut Vec<f64>) {
        let len = self.xs.len();
        self.xs[self.count % len] = x;
        self.count += 1;
        if self.count >= self.w {
            self.emit(self.count - self.w, out);
        }
    }

    fn flush(&mut self, out: &mut Vec<f64>) {
        for j in self.emitted..self.count {
            self.emit(j, out);
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.count = 0;
        self.emitted = 0;
    }
}

/// Running template in phase bins; see the module documentation. A bin with no sample
/// yet uses the mean of its group of `group` bins (the coarse bin).
#[derive(Debug, Clone)]
struct PeriodicTemplate {
    bins: usize,
    group: usize,
    alpha: f64,
    means: Vec<f64>,
    counts: Vec<f64>,
    coarse_means: Vec<f64>,
    coarse_counts: Vec<f64>,
    mean: f64,
    count: f64,
}

impl PeriodicTemplate {
    fn new(bins: usize, group: usize, alpha: f64) -> Self {
        let coarse = if group > 1 { bins.div_ceil(group) } else { 0 };
        Self {
            bins,
            group,
            alpha,
            means: vec![0.0; bins],
            counts: vec![0.0; bins],
            coarse_means: vec![0.0; coarse],
            coarse_counts: vec![0.0; coarse],
            mean: 0.0,
            count: 0.0,
        }
    }

    fn template(&self, bin: usize) -> f64 {
        if self.counts[bin] > 0.0 {
            return self.means[bin] - self.mean;
        }
        if self.group > 1 {
            let coarse = bin / self.group;
            if self.coarse_counts[coarse] > 0.0 {
                return self.coarse_means[coarse] - self.mean;
            }
        }
        0.0
    }

    fn learn(&mut self, x: f64, bin: usize) {
        let bin_count = self.counts[bin] + 1.0;
        self.counts[bin] = bin_count;
        self.means[bin] +=
            (1.0 / bin_count).max(self.alpha * self.bins as f64) * (x - self.means[bin]);
        if self.group > 1 {
            let coarse = bin / self.group;
            let coarse_count = self.coarse_counts[coarse] + 1.0;
            self.coarse_counts[coarse] = coarse_count;
            self.coarse_means[coarse] += (1.0 / coarse_count)
                .max(self.alpha * self.coarse_means.len() as f64)
                * (x - self.coarse_means[coarse]);
        }
        self.count += 1.0;
        self.mean += (1.0 / self.count).max(self.alpha) * (x - self.mean);
    }
}

/// Despike and template stages, on raw volts. Appends outputs in input order: none for
/// the first `despike_window - 1` inputs of a segment, then one per input;
/// [`Self::flush`] appends the rest.
#[derive(Debug, Clone)]
pub struct RawFilterChain {
    plan: FilterPlan,
    despiker: Option<Despiker>,
    template10: Option<PeriodicTemplate>,
    template11: Option<PeriodicTemplate>,
    ratio11: f64,
    index: u64,
    pending: Vec<f64>,
}

impl RawFilterChain {
    pub fn new(plan: &FilterPlan) -> Self {
        let alpha = 1.0 / (TEMPLATE_MEMORY_S * plan.fs);
        Self {
            plan: plan.clone(),
            despiker: (plan.despike_window > 1).then(|| Despiker::new(plan.despike_window)),
            template10: (plan.period10 > 0).then(|| PeriodicTemplate::new(plan.period10, 1, alpha)),
            template11: (plan.period11 > 0.0)
                .then(|| PeriodicTemplate::new(plan.bins11, FRACTIONAL_BINS_PER_SAMPLE, alpha)),
            ratio11: ELEVEN_NINE_HZ / plan.fs,
            index: 0,
            pending: Vec::new(),
        }
    }

    fn finish(&mut self, x: f64, out: &mut Vec<f64>) {
        let n = self.index;
        self.index += 1;
        let bin10 = if self.template10.is_some() {
            (n % self.plan.period10 as u64) as usize
        } else {
            0
        };
        let bin11 = if self.template11.is_some() {
            let turns = n as f64 * self.ratio11;
            let bin = ((turns - turns.floor()) * self.plan.bins11 as f64).floor() as usize;
            bin.min(self.plan.bins11 - 1)
        } else {
            0
        };
        let template10 = self.template10.as_ref().map_or(0.0, |t| t.template(bin10));
        let template11 = self.template11.as_ref().map_or(0.0, |t| t.template(bin11));
        if x.is_finite() {
            if let Some(t) = self.template10.as_mut() {
                t.learn(x - template11, bin10);
            }
            if let Some(t) = self.template11.as_mut() {
                t.learn(x - template10, bin11);
            }
        }
        out.push(x - template10 - template11);
    }

    /// Takes one raw sample (`NaN` for a missing one) and appends the outputs it completes.
    pub fn push(&mut self, x: f64, out: &mut Vec<f64>) {
        let Some(despiker) = self.despiker.as_mut() else {
            self.finish(x, out);
            return;
        };
        let mut pending = std::mem::take(&mut self.pending);
        pending.clear();
        despiker.push(x, &mut pending);
        for &y in &pending {
            self.finish(y, out);
        }
        self.pending = pending;
    }

    /// Appends the outputs still held back by the despike window.
    pub fn flush(&mut self, out: &mut Vec<f64>) {
        let Some(despiker) = self.despiker.as_mut() else {
            return;
        };
        let mut pending = std::mem::take(&mut self.pending);
        pending.clear();
        despiker.flush(&mut pending);
        for &y in &pending {
            self.finish(y, out);
        }
        self.pending = pending;
    }

    /// Ends the segment before `missing` absent samples of the same run, keeping the
    /// templates in phase (the live plot's rule for a long gap).
    #[cfg(test)]
    pub fn break_segment(&mut self, missing: u64, out: &mut Vec<f64>) {
        self.flush(out);
        self.index += missing;
    }
}

/// High-pass then low-pass on calibrated values.
#[derive(Debug, Clone)]
pub struct LinearFilterChain {
    highpass: Option<Biquad>,
    lowpass: Option<Biquad>,
}

impl LinearFilterChain {
    pub fn new(plan: &FilterPlan) -> Self {
        Self {
            highpass: (plan.highpass_hz > 0.0)
                .then(|| Biquad::new(butterworth(true, plan.highpass_hz, plan.fs))),
            lowpass: (plan.lowpass_hz > 0.0)
                .then(|| Biquad::new(butterworth(false, plan.lowpass_hz, plan.fs))),
        }
    }

    pub fn is_active(&self) -> bool {
        self.highpass.is_some() || self.lowpass.is_some()
    }

    pub fn process(&mut self, x: f64) -> f64 {
        let mut y = x;
        if let Some(b) = self.highpass.as_mut() {
            y = b.process(y);
        }
        if let Some(b) = self.lowpass.as_mut() {
            y = b.process(y);
        }
        y
    }

    #[cfg(test)]
    pub fn reset(&mut self) {
        if let Some(b) = self.highpass.as_mut() {
            b.reset();
        }
        if let Some(b) = self.lowpass.as_mut() {
            b.reset();
        }
    }
}

/// Samples the backward pass needs past a sample for the result to have settled:
/// twenty time constants of the lowest cutoff (a 2nd-order Butterworth pole decays
/// with time constant `√2 / (2π fc)`), at least one second and at most two minutes.
pub fn settle_seconds(settings: &ChannelFilterSettings) -> f64 {
    let lowest = [settings.highpass_hz, settings.lowpass_hz]
        .into_iter()
        .flatten()
        .fold(f64::INFINITY, f64::min);
    if !lowest.is_finite() {
        return 1.0;
    }
    (20.0 * std::f64::consts::SQRT_2 / (2.0 * std::f64::consts::PI * lowest)).clamp(1.0, 120.0)
}

/// One queued sample of [`ExportFilter`].
#[derive(Debug)]
struct Entry<T> {
    /// The caller's row, or `None` for a missing sample filled in as `NaN`.
    row: Option<T>,
    /// Fed to the filters (false for a row repeating or preceding the last timestamp).
    fed: bool,
    /// Forward-filtered calibrated value, once known.
    forward: f64,
}

/// A filtered row handed back by [`ExportFilter`].
#[derive(Debug, PartialEq)]
pub struct FilteredRow<T> {
    pub row: T,
    /// Filtered calibrated value; `NaN` when it cannot be computed.
    pub filtered: f64,
}

/// Rows of one channel, in time order, filtered with the zero-phase pipeline.
///
/// Rows go in one at a time with [`Self::push`] and come out, in the same order, with
/// their filtered value once it is settled. The sample interval is taken as the median
/// step of the first rows of each segment. A step of up to
/// [`MAX_FILLED_GAP_SAMPLES`] missing samples is filled with `NaN`; a longer gap, a
/// step that is not close to a whole number of intervals, or a new rate ends the
/// segment and every filter (templates included) starts over. Rows whose timestamp is
/// not after the previous one are passed through with `NaN`.
///
/// Memory stays bounded: once `block + margin` samples wait, the backward pass runs
/// from the newest sample (its state settled over `margin` samples) and the oldest
/// `block` rows come out.
pub struct ExportFilter<T> {
    settings: ChannelFilterSettings,
    block: usize,
    /// Rows waiting for the interval to be estimated.
    probe: VecDeque<(i64, f64, T)>,
    segment: Option<Segment>,
    queue: VecDeque<Entry<T>>,
    /// Entries at the front of `queue` whose forward value is known.
    ready: usize,
    scratch: Vec<f64>,
}

/// Filter state of one contiguous segment.
struct Segment {
    interval_ns: i64,
    last_ns: i64,
    raw: RawFilterChain,
    forward: LinearFilterChain,
    plan: FilterPlan,
    margin: usize,
}

/// Rows used to estimate the sample interval of a segment.
const PROBE_ROWS: usize = 32;

impl<T> ExportFilter<T> {
    /// Default rows per backward pass.
    pub const BLOCK: usize = 65_536;

    pub fn new(settings: ChannelFilterSettings) -> Self {
        Self::with_block(settings, Self::BLOCK)
    }

    pub fn with_block(settings: ChannelFilterSettings, block: usize) -> Self {
        Self {
            settings,
            block: block.max(1),
            probe: VecDeque::new(),
            segment: None,
            queue: VecDeque::new(),
            ready: 0,
            scratch: Vec::new(),
        }
    }

    /// Adds one row and appends the rows that are done to `out`.
    ///
    /// * `timestamp_ns` - Sample time.
    /// * `raw` - Raw value.
    /// * `row` - The caller's row, handed back with the filtered value.
    /// * `calibrate` - Converts a (filtered) raw value of a row into its unit.
    pub fn push(
        &mut self,
        timestamp_ns: i64,
        raw: f64,
        row: T,
        calibrate: &dyn Fn(&T, f64) -> f64,
        out: &mut Vec<FilteredRow<T>>,
    ) {
        if self.segment.is_none() {
            self.probe.push_back((timestamp_ns, raw, row));
            if self.probe.len() >= PROBE_ROWS {
                self.start_from_probe(calibrate, out);
            }
            return;
        }
        self.push_in_segment(timestamp_ns, raw, row, calibrate, out);
    }

    /// Filters and hands back every row still held.
    pub fn finish(&mut self, calibrate: &dyn Fn(&T, f64) -> f64, out: &mut Vec<FilteredRow<T>>) {
        while !self.probe.is_empty() {
            self.start_from_probe(calibrate, out);
        }
        self.end_segment(calibrate, out);
    }

    /// Starts a segment with the interval estimated from the probe rows, then feeds them.
    fn start_from_probe(
        &mut self,
        calibrate: &dyn Fn(&T, f64) -> f64,
        out: &mut Vec<FilteredRow<T>>,
    ) {
        let mut steps: Vec<i64> = self
            .probe
            .iter()
            .zip(self.probe.iter().skip(1))
            .map(|(a, b)| b.0 - a.0)
            .filter(|step| *step > 0)
            .collect();
        steps.sort_unstable();
        let Some(&interval_ns) = steps.get(steps.len() / 2) else {
            // Too few rows to tell the rate: pass them through unfiltered.
            for (_, _, row) in self.probe.drain(..) {
                out.push(FilteredRow {
                    row,
                    filtered: f64::NAN,
                });
            }
            return;
        };
        let rows: Vec<(i64, f64, T)> = self.probe.drain(..).collect();
        let mut rows = rows.into_iter();
        if let Some((first_ns, raw, row)) = rows.next() {
            self.begin_segment(interval_ns, first_ns);
            self.feed(raw, Some(row), calibrate);
        }
        for (timestamp_ns, raw, row) in rows {
            if self.segment.is_none() {
                self.probe.push_back((timestamp_ns, raw, row));
            } else {
                self.push_in_segment(timestamp_ns, raw, row, calibrate, out);
            }
        }
        self.drain_settled(calibrate, out, false);
    }

    fn begin_segment(&mut self, interval_ns: i64, first_ns: i64) {
        let fs = 1e9 / interval_ns as f64;
        let plan = plan_filters(&self.settings, fs);
        let margin = (settle_seconds(&self.settings) * fs).ceil() as usize;
        self.segment = Some(Segment {
            interval_ns,
            last_ns: first_ns,
            raw: RawFilterChain::new(&plan),
            forward: LinearFilterChain::new(&plan),
            plan,
            margin,
        });
    }

    fn push_in_segment(
        &mut self,
        timestamp_ns: i64,
        raw: f64,
        row: T,
        calibrate: &dyn Fn(&T, f64) -> f64,
        out: &mut Vec<FilteredRow<T>>,
    ) {
        let segment = self.segment.as_ref().expect("segment");
        let step = timestamp_ns - segment.last_ns;
        if step <= 0 {
            self.queue.push_back(Entry {
                row: Some(row),
                fed: false,
                forward: f64::NAN,
            });
            self.advance_ready();
            return;
        }
        let interval = segment.interval_ns;
        let intervals = (step + interval / 2) / interval;
        let off_grid = (step - intervals * interval).abs() > interval / 2;
        let missing = intervals - 1;
        if off_grid || missing > MAX_FILLED_GAP_SAMPLES {
            self.end_segment(calibrate, out);
            self.probe.push_back((timestamp_ns, raw, row));
            return;
        }
        for _ in 0..missing {
            self.feed(f64::NAN, None, calibrate);
        }
        self.segment.as_mut().expect("segment").last_ns = timestamp_ns;
        self.feed(raw, Some(row), calibrate);
        self.drain_settled(calibrate, out, false);
    }

    /// Queues one sample and runs it through the causal part.
    fn feed(&mut self, raw: f64, row: Option<T>, calibrate: &dyn Fn(&T, f64) -> f64) {
        self.queue.push_back(Entry {
            row,
            fed: true,
            forward: f64::NAN,
        });
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        self.segment
            .as_mut()
            .expect("segment")
            .raw
            .push(raw, &mut scratch);
        self.apply_forward(&scratch, calibrate);
        self.scratch = scratch;
    }

    /// Stores forward values for the oldest fed entries that lack one.
    fn apply_forward(&mut self, values: &[f64], calibrate: &dyn Fn(&T, f64) -> f64) {
        let segment = self.segment.as_mut().expect("segment");
        for &value in values {
            while !self.queue[self.ready].fed {
                self.ready += 1;
            }
            let entry = &mut self.queue[self.ready];
            let calibrated = match &entry.row {
                Some(row) => calibrate(row, value),
                None => f64::NAN,
            };
            entry.forward = segment.forward.process(calibrated);
            self.ready += 1;
        }
        self.advance_ready();
    }

    /// Moves `ready` over unfed entries that directly follow the ready ones.
    fn advance_ready(&mut self) {
        while self.ready < self.queue.len() && !self.queue[self.ready].fed {
            self.ready += 1;
        }
    }

    /// Runs the backward pass and hands back settled rows: `block` of them once
    /// `block + margin` are ready, or all of them when `all` is set.
    fn drain_settled(
        &mut self,
        _calibrate: &dyn Fn(&T, f64) -> f64,
        out: &mut Vec<FilteredRow<T>>,
        all: bool,
    ) {
        let Some(segment) = self.segment.as_ref() else {
            return;
        };
        let count = if all {
            self.ready
        } else if self.ready >= self.block + segment.margin {
            self.block
        } else {
            return;
        };
        let mut backward = LinearFilterChain::new(&segment.plan);
        let zero_phase = backward.is_active();
        let mut values = std::mem::take(&mut self.scratch);
        values.clear();
        values.resize(self.ready, f64::NAN);
        for i in (0..self.ready).rev() {
            let entry = &self.queue[i];
            values[i] = if !entry.fed {
                f64::NAN
            } else if zero_phase {
                backward.process(entry.forward)
            } else {
                entry.forward
            };
        }
        for value in values.iter().take(count) {
            let entry = self.queue.pop_front().expect("entry");
            if let Some(row) = entry.row {
                out.push(FilteredRow {
                    row,
                    filtered: *value,
                });
            }
        }
        self.ready -= count;
        self.scratch = values;
    }

    /// Flushes the causal part, hands back every queued row and forgets the segment.
    fn end_segment(&mut self, calibrate: &dyn Fn(&T, f64) -> f64, out: &mut Vec<FilteredRow<T>>) {
        if self.segment.is_none() {
            return;
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        self.segment
            .as_mut()
            .expect("segment")
            .raw
            .flush(&mut scratch);
        self.apply_forward(&scratch, calibrate);
        self.scratch = scratch;
        self.drain_settled(calibrate, out, true);
        debug_assert!(self.queue.is_empty());
        self.queue.clear();
        self.ready = 0;
        self.segment = None;
    }
}

/// Runs the causal pipeline over an array with no calibration, as the webapp's
/// `filterCausal` does (a gap longer than [`MAX_FILLED_GAP_SAMPLES`] breaks the segment
/// but keeps the templates). Used to compare with the shared vectors.
#[cfg(test)]
pub fn filter_causal(values: &[f64], plan: &FilterPlan) -> Vec<f64> {
    let mut raw = RawFilterChain::new(plan);
    let mut linear = LinearFilterChain::new(plan);
    let mut result = Vec::with_capacity(values.len());
    let mut out = Vec::new();
    let mut i = 0;
    while i < values.len() {
        if values[i].is_finite() {
            raw.push(values[i], &mut out);
            result.extend(out.drain(..).map(|y| linear.process(y)));
            i += 1;
            continue;
        }
        let mut end = i;
        while end < values.len() && !values[end].is_finite() {
            end += 1;
        }
        let missing = end - i;
        if missing as i64 <= MAX_FILLED_GAP_SAMPLES {
            for _ in 0..missing {
                raw.push(f64::NAN, &mut out);
            }
            result.extend(out.drain(..).map(|y| linear.process(y)));
        } else {
            raw.break_segment(missing as u64, &mut out);
            result.extend(out.drain(..).map(|y| linear.process(y)));
            result.extend(std::iter::repeat_n(f64::NAN, missing));
            linear.reset();
        }
        i = end;
    }
    raw.flush(&mut out);
    result.extend(out.drain(..).map(|y| linear.process(y)));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Vectors {
        cases: Vec<VectorCase>,
    }

    #[derive(serde::Deserialize)]
    struct VectorCase {
        name: String,
        fs: f64,
        settings: Value,
        input_uv: Vec<Option<f64>>,
        squares_uv: Vec<f64>,
        spikes: Vec<usize>,
        spike_uv: Vec<f64>,
        pulses: Vec<usize>,
        pulse_uv: f64,
        warmup: usize,
        gaps: Vec<(usize, usize)>,
        expected_causal_uv: Vec<Option<f64>>,
    }

    fn vectors() -> Vec<VectorCase> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../testdata/filter-vectors.json"
        );
        let text = std::fs::read_to_string(path).expect("read testdata/filter-vectors.json");
        serde_json::from_str::<Vectors>(&text)
            .expect("parse vectors")
            .cases
    }

    fn volts(values: &[Option<f64>]) -> Vec<f64> {
        values
            .iter()
            .map(|v| v.map_or(f64::NAN, |uv| uv / 1e6))
            .collect()
    }

    /// Runs the export filter (identity calibration) over values with `NaN` for
    /// missing samples; returns one output per input position.
    fn export_filter(case: &VectorCase, values: &[f64], block: usize) -> Vec<f64> {
        let settings = ChannelFilterSettings::from_value(&case.settings);
        let interval = (1e9 / case.fs).round() as i64;
        let mut filter = ExportFilter::with_block(settings, block);
        let identity = |_: &usize, v: f64| v;
        let mut out = Vec::new();
        for (i, v) in values.iter().enumerate() {
            if v.is_finite() {
                filter.push(i as i64 * interval, *v, i, &identity, &mut out);
            }
        }
        filter.finish(&identity, &mut out);
        let mut result = vec![f64::NAN; values.len()];
        let mut last = None;
        for row in out {
            assert!(last.is_none_or(|l| l < row.row), "rows out of order");
            last = Some(row.row);
            result[row.row] = row.filtered;
        }
        result
    }

    #[test]
    fn causal_pipeline_matches_the_webapp_vectors() {
        for case in vectors() {
            let plan = plan_filters(&ChannelFilterSettings::from_value(&case.settings), case.fs);
            let output = filter_causal(&volts(&case.input_uv), &plan);
            assert_eq!(output.len(), case.expected_causal_uv.len(), "{}", case.name);
            let mut worst: f64 = 0.0;
            for (i, (got, want)) in output.iter().zip(&case.expected_causal_uv).enumerate() {
                match want {
                    None => assert!(
                        got.is_nan(),
                        "{} sample {i}: {got} should be NaN",
                        case.name
                    ),
                    Some(uv) => {
                        let want = uv / 1e6;
                        let scale = want.abs().max(got.abs()).max(1e-3);
                        let error = (got - want).abs() / scale;
                        worst = worst.max(error);
                        assert!(error <= 1e-9, "{} sample {i}: {got} vs {want}", case.name);
                    }
                }
            }
            println!("{}: worst relative difference {worst:e}", case.name);
        }
    }

    /// What the zero-phase export leaves of the square waves (dB below them), of the
    /// pulses (fraction of the height kept) and of the spikes (how many were checked,
    /// and dB below them, against a run without them).
    fn quality(case: &VectorCase, block: usize) -> (f64, Vec<f64>, (usize, f64)) {
        let full = export_filter(case, &volts(&case.input_uv), block);
        let without_squares: Vec<Option<f64>> = case
            .input_uv
            .iter()
            .zip(&case.squares_uv)
            .map(|(v, s)| v.map(|v| v - s))
            .collect();
        let clean = export_filter(case, &volts(&without_squares), block);
        let mut without_spikes = case.input_uv.clone();
        for (k, &s) in case.spikes.iter().enumerate() {
            if let Some(v) = without_spikes[s].as_mut() {
                *v -= case.spike_uv[k];
            }
        }
        let smooth = export_filter(case, &volts(&without_spikes), block);

        let fs = case.fs;
        let mut mask: Vec<bool> = case
            .input_uv
            .iter()
            .enumerate()
            .map(|(i, v)| v.is_some() && i >= case.warmup)
            .collect();
        let n = mask.len();
        let mut clear = |from: f64, to: f64| {
            for m in mask
                .iter_mut()
                .take((to.max(0.0) as usize).min(n))
                .skip(from.max(0.0) as usize)
            {
                *m = false;
            }
        };
        for &p in &case.pulses {
            clear(p as f64 - 0.08 * fs, p as f64 + 0.08 * fs);
        }
        for &(start, length) in &case.gaps {
            // Every filter, the templates included, restarts after a long gap in an
            // export; they need about 2000 samples to learn again.
            let after = if length as i64 > MAX_FILLED_GAP_SAMPLES {
                2000.0 / fs
            } else {
                0.3
            };
            clear(
                start as f64 - 0.05 * fs,
                (start + length) as f64 + after * fs,
            );
        }
        let (mut power, mut residual) = (0.0, 0.0);
        for i in 0..n {
            if mask[i] {
                power += (case.squares_uv[i] / 1e6).powi(2);
                residual += (full[i] - clean[i]).powi(2);
            }
        }
        let pulses = case
            .pulses
            .iter()
            .map(|&p| {
                let w = (0.01 * fs).round() as usize;
                let peak = full[p - w..=p + w].iter().cloned().fold(f64::MIN, f64::max);
                let mut base: Vec<f64> = full
                    [p - (0.07 * fs).round() as usize..p - (0.03 * fs).round() as usize]
                    .to_vec();
                base.sort_by(f64::total_cmp);
                (peak - base[base.len() / 2]) / (case.pulse_uv / 1e6)
            })
            .collect();
        // A spike next to a square-wave edge merges into the edge (the opening moves
        // the edge by a sample), so only spikes on a flat stretch count.
        let (mut spike_power, mut spike_residual, mut spike_count) = (0.0, 0.0, 0);
        for (k, &s) in case.spikes.iter().enumerate() {
            if mask[s] && (s - 6..=s + 6).all(|i| case.squares_uv[i] == case.squares_uv[s]) {
                spike_count += 1;
                spike_power += (case.spike_uv[k] / 1e6).powi(2);
                spike_residual += (full[s] - smooth[s]).powi(2);
            }
        }
        let spikes = (spike_count, 10.0 * (spike_power / spike_residual).log10());
        (10.0 * (power / residual).log10(), pulses, spikes)
    }

    #[test]
    fn zero_phase_export_removes_the_interference_and_keeps_the_pulses() {
        for case in vectors() {
            let plan = plan_filters(&ChannelFilterSettings::from_value(&case.settings), case.fs);
            let (reduction, pulses, spikes) = quality(&case, ExportFilter::<usize>::BLOCK);
            println!(
                "{}: square waves {reduction:.1} dB down, pulses kept {pulses:.3?}, {} spikes {:.1} dB down",
                case.name, spikes.0, spikes.1
            );
            assert!(reduction >= 25.0, "{}: {reduction} dB", case.name);
            for kept in pulses {
                assert!((kept - 1.0).abs() < 0.1, "{}: pulse kept {kept}", case.name);
            }
            if plan.despike_window > 0 {
                assert!(spikes.0 > 50);
                assert!(
                    spikes.1 >= 20.0,
                    "{}: spikes {} dB down",
                    case.name,
                    spikes.1
                );
            }
        }
    }

    #[test]
    fn small_blocks_give_the_same_export_as_one_pass() {
        for case in vectors() {
            let values = volts(&case.input_uv);
            let whole = export_filter(&case, &values, usize::MAX / 4);
            let blocks = export_filter(&case, &values, 997);
            let scale = whole
                .iter()
                .filter(|v| v.is_finite())
                .fold(1e-3_f64, |a, b| a.max(b.abs()));
            for (i, (a, b)) in whole.iter().zip(&blocks).enumerate() {
                assert!(
                    (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-9 * scale,
                    "{} sample {i}: {a} vs {b}",
                    case.name
                );
            }
        }
    }

    #[test]
    fn export_rows_come_back_once_in_order_with_repeats_unfiltered() {
        let settings = ChannelFilterSettings {
            despike: true,
            lowpass_hz: Some(100.0),
            ..Default::default()
        };
        let mut filter = ExportFilter::with_block(settings, 64);
        let identity = |_: &usize, v: f64| v;
        let mut out = Vec::new();
        let mut index = 0;
        for i in 0..1000_i64 {
            filter.push(i * 500_000, 1.0, index, &identity, &mut out);
            index += 1;
            if i == 500 {
                // Same timestamp again: passed through without a filtered value.
                filter.push(i * 500_000, 2.0, index, &identity, &mut out);
                index += 1;
            }
        }
        // A new run at another rate after a long gap.
        for i in 0..100_i64 {
            filter.push(
                10_000_000_000 + i * 10_000_000,
                1.0,
                index,
                &identity,
                &mut out,
            );
            index += 1;
        }
        filter.finish(&identity, &mut out);
        let order: Vec<usize> = out.iter().map(|r| r.row).collect();
        assert_eq!(order, (0..index).collect::<Vec<_>>());
        assert!(out[501].filtered.is_nan());
        for row in out.iter().filter(|r| r.row != 501) {
            assert!(
                (row.filtered - 1.0).abs() < 1e-12,
                "row {}: {}",
                row.row,
                row.filtered
            );
        }
    }

    #[test]
    fn settings_are_read_like_the_webapp() {
        let value: Value = serde_json::json!({
            "despike": true, "remove_10hz": "yes", "remove_11_9hz": null,
            "highpass_hz": "1.5", "lowpass_hz": 0
        });
        let settings = ChannelFilterSettings::from_value(&value);
        assert_eq!(
            settings,
            ChannelFilterSettings {
                despike: true,
                highpass_hz: Some(1.5),
                ..Default::default()
            }
        );
        let plan = plan_filters(&settings, 2000.0);
        assert_eq!(plan.despike_window, 5);
        let all = ChannelFilterSettings {
            despike: true,
            remove_10hz: true,
            remove_11_9hz: true,
            highpass_hz: Some(1.0),
            lowpass_hz: Some(100.0),
        };
        let plan = plan_filters(&all, 100.0);
        assert_eq!(
            (plan.despike_window, plan.period10, plan.lowpass_hz),
            (0, 10, 0.0)
        );
        assert_eq!(plan_filters(&all, 2000.0).bins11, 672);
        assert_eq!(plan_filters(&all, 2205.0).period10, 0);
    }
}
