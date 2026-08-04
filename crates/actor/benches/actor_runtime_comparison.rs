use std::{
    hint::black_box,
    io::{self, Write},
    time::{Duration, Instant},
};

use actix::Actor as _;
use loong_actor::{ActorOwner, ActorRef, ExitReason, Shutdown, prelude::*, spawn};
use oorandom::Rand64;
use serde::Serialize;

const MAILBOX_CAPACITY: usize = 64;
const ONE_WAY_BATCH: usize = 32;
const PROVISIONAL_TARGET: Duration = Duration::from_millis(5);
const MEASUREMENT_TARGET: Duration = Duration::from_millis(10);
const WARM_UP_BLOCKS: usize = 8;
const ORDER_BLOCKS: usize = 50;
const RESAMPLING_UNITS: usize = ORDER_BLOCKS / 2;
const MAX_ITERATIONS: u64 = 100_000_000;
const BOOTSTRAP_RESAMPLES: usize = 100_000;
const BOOTSTRAP_SEED: u64 = 0x100a_2026;
const CONFIDENCE_LEVEL: f64 = 0.95;

#[derive(Message)]
#[message(reply = u64)]
struct Ready;

impl actix::Message for Ready {
    type Result = u64;
}

#[derive(Message)]
struct Notify;

impl actix::Message for Notify {
    type Result = ();
}

#[derive(Message)]
#[message(reply = u64)]
struct Barrier;

impl actix::Message for Barrier {
    type Result = u64;
}

struct LoongActor {
    handled: u64,
}

#[loong_actor::actor(mailbox = MAILBOX_CAPACITY)]
impl Actor for LoongActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { handled: 0 }
    }
}

impl SyncHandler<Ready> for LoongActor {
    fn handle(&mut self, _message: Ready, _scope: &mut ActorScope<Self>) -> u64 {
        1
    }
}

impl SyncHandler<Notify> for LoongActor {
    fn handle(&mut self, _message: Notify, _scope: &mut ActorScope<Self>) {
        self.handled += 1;
    }
}

impl SyncHandler<Barrier> for LoongActor {
    fn handle(&mut self, _message: Barrier, _scope: &mut ActorScope<Self>) -> u64 {
        self.handled
    }
}

struct ActixActor {
    handled: u64,
}

impl actix::Actor for ActixActor {
    type Context = actix::Context<Self>;

    fn started(&mut self, context: &mut Self::Context) {
        context.set_mailbox_capacity(MAILBOX_CAPACITY);
    }
}

impl actix::Handler<Ready> for ActixActor {
    type Result = u64;

    fn handle(&mut self, _message: Ready, _context: &mut Self::Context) -> Self::Result {
        1
    }
}

impl actix::Handler<Notify> for ActixActor {
    type Result = ();

    fn handle(&mut self, _message: Notify, _context: &mut Self::Context) -> Self::Result {
        self.handled += 1;
    }
}

impl actix::Handler<Barrier> for ActixActor {
    type Result = u64;

    fn handle(&mut self, _message: Barrier, _context: &mut Self::Context) -> Self::Result {
        self.handled
    }
}

// Both workloads measure public end-to-end message paths.
// Runtime construction and teardown stay outside timed legs.
// Different lifecycle guarantees remain part of measured costs.
#[derive(Clone, Copy)]
enum Workload {
    ReadyRequestReply,
    // A FIFO barrier drains every notification batch.
    // This does not measure saturation or backpressure.
    OneWayHandlerDrain,
}

impl Workload {
    const ALL: [Self; 2] = [Self::ReadyRequestReply, Self::OneWayHandlerDrain];

    const fn name(self) -> &'static str {
        match self {
            Self::ReadyRequestReply => "ready_request_reply",
            Self::OneWayHandlerDrain => "one_way_handler_drain",
        }
    }

    const fn definition(self) -> &'static str {
        match self {
            Self::ReadyRequestReply => "one bounded call and one ready reply",
            Self::OneWayHandlerDrain => "32 try_send notifications followed by one barrier call",
        }
    }

    const fn boundary(self) -> &'static str {
        match self {
            Self::ReadyRequestReply => "mailbox saturation is outside this workload",
            Self::OneWayHandlerDrain => {
                "barrier cost is amortized; saturation and backpressure are excluded"
            }
        }
    }

    const fn element(self) -> &'static str {
        match self {
            Self::ReadyRequestReply => "request",
            Self::OneWayHandlerDrain => "notification",
        }
    }

    const fn elements_per_iteration(self) -> u64 {
        match self {
            Self::ReadyRequestReply => 1,
            Self::OneWayHandlerDrain => ONE_WAY_BATCH as u64,
        }
    }

    const fn seed_offset(self) -> u64 {
        match self {
            Self::ReadyRequestReply => 0,
            Self::OneWayHandlerDrain => 1,
        }
    }
}

#[derive(Clone, Copy)]
enum Implementation {
    Loong,
    Actix,
}

struct RuntimePair {
    loong_runtime: tokio::runtime::Runtime,
    loong_owner: ActorOwner<LoongActor>,
    loong_actor: ActorRef<LoongActor>,
    loong_handled: u64,
    actix_system: actix::SystemRunner,
    actix_actor: actix::Addr<ActixActor>,
    actix_handled: u64,
}

impl RuntimePair {
    fn new() -> Self {
        let loong_runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("the Loong benchmark runtime builds");
        let loong_owner = loong_runtime.block_on(async { spawn::<LoongActor>(()) });
        let loong_actor = loong_owner.actor_ref();

        let actix_system = actix::System::with_tokio_rt(|| {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("the Actix benchmark runtime builds")
        });
        let actix_actor = actix_system.block_on(async { ActixActor { handled: 0 }.start() });

        assert_eq!(
            loong_runtime
                .block_on(loong_actor.call(Ready))
                .expect("the Loong actor starts"),
            1
        );
        assert_eq!(
            actix_system
                .block_on(actix_actor.send(Ready))
                .expect("the Actix actor starts"),
            1
        );
        Self {
            loong_runtime,
            loong_owner,
            loong_actor,
            loong_handled: 0,
            actix_system,
            actix_actor,
            actix_handled: 0,
        }
    }

    fn measure(
        &mut self,
        implementation: Implementation,
        workload: Workload,
        iterations: u64,
    ) -> Duration {
        match implementation {
            Implementation::Loong => self.measure_loong(workload, iterations),
            Implementation::Actix => self.measure_actix(workload, iterations),
        }
    }

    fn measure_loong(&mut self, workload: Workload, iterations: u64) -> Duration {
        let actor = &self.loong_actor;
        match workload {
            Workload::ReadyRequestReply => self.loong_runtime.block_on(async {
                let started = Instant::now();
                for _ in 0..iterations {
                    let reply = actor
                        .call(Ready)
                        .await
                        .expect("the Loong actor stays alive");
                    black_box(reply);
                }
                started.elapsed()
            }),
            Workload::OneWayHandlerDrain => {
                let expected = self.loong_handled + iterations * ONE_WAY_BATCH as u64;
                let (elapsed, handled) = self.loong_runtime.block_on(async {
                    let mut handled = self.loong_handled;
                    let started = Instant::now();
                    for _ in 0..iterations {
                        for _ in 0..ONE_WAY_BATCH {
                            actor
                                .try_send(Notify)
                                .expect("the Loong mailbox accepts one batch");
                        }
                        handled = actor
                            .call(Barrier)
                            .await
                            .expect("the Loong barrier completes");
                        black_box(handled);
                    }
                    (started.elapsed(), handled)
                });
                assert_eq!(handled, expected);
                self.loong_handled = handled;
                elapsed
            }
        }
    }

    fn measure_actix(&mut self, workload: Workload, iterations: u64) -> Duration {
        let actor = &self.actix_actor;
        match workload {
            Workload::ReadyRequestReply => self.actix_system.block_on(async {
                let started = Instant::now();
                for _ in 0..iterations {
                    let reply = actor
                        .send(Ready)
                        .await
                        .expect("the Actix actor stays alive");
                    black_box(reply);
                }
                started.elapsed()
            }),
            Workload::OneWayHandlerDrain => {
                let expected = self.actix_handled + iterations * ONE_WAY_BATCH as u64;
                let (elapsed, handled) = self.actix_system.block_on(async {
                    let mut handled = self.actix_handled;
                    let started = Instant::now();
                    for _ in 0..iterations {
                        for _ in 0..ONE_WAY_BATCH {
                            actor
                                .try_send(Notify)
                                .expect("the Actix mailbox accepts one batch");
                        }
                        handled = actor
                            .send(Barrier)
                            .await
                            .expect("the Actix barrier completes");
                        black_box(handled);
                    }
                    (started.elapsed(), handled)
                });
                assert_eq!(handled, expected);
                self.actix_handled = handled;
                elapsed
            }
        }
    }

    fn shutdown(self) {
        let status = self
            .loong_runtime
            .block_on(self.loong_owner.shutdown(Shutdown::Kill));
        assert_eq!(status.reason(), ExitReason::Killed);

        drop(self.actix_actor);
        actix::System::current().stop();
        self.actix_system
            .run()
            .expect("the Actix benchmark system stops cleanly");
    }
}

#[derive(Clone, Copy)]
struct RoundSample {
    loong: Duration,
    actix: Duration,
}

impl RoundSample {
    fn log_ratio(self) -> f64 {
        (self.loong.as_secs_f64() / self.actix.as_secs_f64()).ln()
    }
}

#[derive(Clone, Copy)]
struct OrderBlock {
    loong_first: RoundSample,
    actix_first: RoundSample,
}

impl OrderBlock {
    fn log_ratio(self) -> f64 {
        (self.loong_first.log_ratio() + self.actix_first.log_ratio()) / 2.0
    }
}

#[derive(Serialize)]
struct ConfidenceInterval {
    lower: f64,
    upper: f64,
}

#[derive(Serialize)]
struct WorkloadReport {
    name: &'static str,
    definition: &'static str,
    boundary: &'static str,
    element: &'static str,
    elements_per_iteration: u64,
    iterations_per_leg: u64,
    loong_geometric_mean_ns_per_element: f64,
    actix_geometric_mean_ns_per_element: f64,
    geometric_mean_ratio: f64,
    loong_first_geometric_mean_ratio: f64,
    actix_first_geometric_mean_ratio: f64,
    bootstrap_seed: u64,
    paired_superblock_bootstrap_confidence_interval: ConfidenceInterval,
}

#[derive(Serialize)]
struct BenchmarkReport {
    schema: &'static str,
    ratio_definition: &'static str,
    ratio_interpretation: &'static str,
    measurement_scope: &'static str,
    comparison_limit: &'static str,
    confidence_level: f64,
    confidence_interval_method: &'static str,
    outlier_policy: &'static str,
    bootstrap_resamples: usize,
    order_blocks: usize,
    rounds_per_block: usize,
    resampling_units: usize,
    workloads: Vec<WorkloadReport>,
}

fn measure_round(
    runtimes: &mut RuntimePair,
    workload: Workload,
    iterations: u64,
    first: Implementation,
) -> RoundSample {
    match first {
        Implementation::Loong => {
            let loong = runtimes.measure(Implementation::Loong, workload, iterations);
            let actix = runtimes.measure(Implementation::Actix, workload, iterations);
            RoundSample { loong, actix }
        }
        Implementation::Actix => {
            let actix = runtimes.measure(Implementation::Actix, workload, iterations);
            let loong = runtimes.measure(Implementation::Loong, workload, iterations);
            RoundSample { loong, actix }
        }
    }
}

// Each block contains both orders.
// Reversing their sequence also balances block-boundary drift.
fn measure_order_block(
    runtimes: &mut RuntimePair,
    workload: Workload,
    iterations: u64,
    reverse: bool,
) -> OrderBlock {
    if reverse {
        let actix_first = measure_round(runtimes, workload, iterations, Implementation::Actix);
        let loong_first = measure_round(runtimes, workload, iterations, Implementation::Loong);
        OrderBlock {
            loong_first,
            actix_first,
        }
    } else {
        let loong_first = measure_round(runtimes, workload, iterations, Implementation::Loong);
        let actix_first = measure_round(runtimes, workload, iterations, Implementation::Actix);
        OrderBlock {
            loong_first,
            actix_first,
        }
    }
}

// Calibration uses the faster leg.
// Both measured legs must clear the timer-noise target.
fn calibrate(
    runtimes: &mut RuntimePair,
    workload: Workload,
    target: Duration,
    initial_iterations: u64,
) -> u64 {
    let mut iterations = initial_iterations.max(1);
    loop {
        let sample = measure_round(runtimes, workload, iterations, Implementation::Loong);
        let fastest = sample.loong.min(sample.actix);
        if fastest >= target || iterations == MAX_ITERATIONS {
            return iterations;
        }

        let next = if fastest.is_zero() {
            iterations.saturating_mul(10)
        } else {
            let scale = target.as_secs_f64() / fastest.as_secs_f64();
            ((iterations as f64 * scale * 1.05).ceil() as u64).max(iterations + 1)
        };
        iterations = next.min(MAX_ITERATIONS);
    }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn percentile(sorted: &[f64], probability: f64) -> f64 {
    let rank = probability * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let fraction = rank - lower as f64;
    sorted[lower] + (sorted[upper] - sorted[lower]) * fraction
}

// Resampling superblocks preserves both alternating block orientations.
fn bootstrap_interval(superblock_log_ratios: &[f64], seed: u64) -> ConfidenceInterval {
    let mut random = Rand64::new(u128::from(seed));
    let mut distribution = Vec::with_capacity(BOOTSTRAP_RESAMPLES);
    for _ in 0..BOOTSTRAP_RESAMPLES {
        let mut total = 0.0;
        for _ in superblock_log_ratios {
            let index = random.rand_range(0..superblock_log_ratios.len() as u64) as usize;
            total += superblock_log_ratios[index];
        }
        distribution.push(total / superblock_log_ratios.len() as f64);
    }
    distribution.sort_unstable_by(f64::total_cmp);

    let tail = (1.0 - CONFIDENCE_LEVEL) / 2.0;
    ConfidenceInterval {
        lower: percentile(&distribution, tail).exp(),
        upper: percentile(&distribution, 1.0 - tail).exp(),
    }
}

fn analyze(workload: Workload, iterations: u64, blocks: &[OrderBlock]) -> WorkloadReport {
    let elements = (iterations * workload.elements_per_iteration()) as f64;
    let mut loong_logs = Vec::with_capacity(blocks.len() * 2);
    let mut actix_logs = Vec::with_capacity(blocks.len() * 2);
    let mut loong_first_logs = Vec::with_capacity(blocks.len());
    let mut actix_first_logs = Vec::with_capacity(blocks.len());
    let mut block_logs = Vec::with_capacity(blocks.len());

    for block in blocks {
        for sample in [block.loong_first, block.actix_first] {
            loong_logs.push((sample.loong.as_secs_f64() * 1e9 / elements).ln());
            actix_logs.push((sample.actix.as_secs_f64() * 1e9 / elements).ln());
        }
        loong_first_logs.push(block.loong_first.log_ratio());
        actix_first_logs.push(block.actix_first.log_ratio());
        block_logs.push(block.log_ratio());
    }
    assert_eq!(block_logs.len() % 2, 0);
    let superblock_logs = block_logs
        .chunks_exact(2)
        .map(|pair| (pair[0] + pair[1]) / 2.0)
        .collect::<Vec<_>>();
    assert_eq!(superblock_logs.len(), RESAMPLING_UNITS);
    let bootstrap_seed = BOOTSTRAP_SEED.wrapping_add(workload.seed_offset());

    WorkloadReport {
        name: workload.name(),
        definition: workload.definition(),
        boundary: workload.boundary(),
        element: workload.element(),
        elements_per_iteration: workload.elements_per_iteration(),
        iterations_per_leg: iterations,
        loong_geometric_mean_ns_per_element: mean(&loong_logs).exp(),
        actix_geometric_mean_ns_per_element: mean(&actix_logs).exp(),
        geometric_mean_ratio: mean(&block_logs).exp(),
        loong_first_geometric_mean_ratio: mean(&loong_first_logs).exp(),
        actix_first_geometric_mean_ratio: mean(&actix_first_logs).exp(),
        bootstrap_seed,
        paired_superblock_bootstrap_confidence_interval: bootstrap_interval(
            &superblock_logs,
            bootstrap_seed,
        ),
    }
}

fn measure_workload(runtimes: &mut RuntimePair, workload: Workload) -> WorkloadReport {
    let provisional = calibrate(runtimes, workload, PROVISIONAL_TARGET, 1);
    for index in 0..WARM_UP_BLOCKS {
        let _ = measure_order_block(runtimes, workload, provisional, index % 2 == 1);
    }
    let iterations = calibrate(runtimes, workload, MEASUREMENT_TARGET, provisional);
    eprintln!(
        "measuring {} with {iterations} iterations per leg",
        workload.name()
    );

    let mut blocks = Vec::with_capacity(ORDER_BLOCKS);
    for index in 0..ORDER_BLOCKS {
        blocks.push(measure_order_block(
            runtimes,
            workload,
            iterations,
            index % 2 == 1,
        ));
    }
    analyze(workload, iterations, &blocks)
}

fn main() {
    let mut runtimes = RuntimePair::new();
    let workloads = Workload::ALL
        .into_iter()
        .map(|workload| measure_workload(&mut runtimes, workload))
        .collect();
    runtimes.shutdown();

    let report = BenchmarkReport {
        schema: "loong.actor_runtime_ratio.v1",
        ratio_definition: "elapsed_time.loong / elapsed_time.actix",
        ratio_interpretation: "values below 1 mean Loong is faster",
        measurement_scope: "end-to-end public message paths; setup and teardown are excluded",
        comparison_limit: "different lifecycle guarantees remain included",
        confidence_level: CONFIDENCE_LEVEL,
        confidence_interval_method: "percentile bootstrap over adjacent reversed block pairs",
        outlier_policy: "none",
        bootstrap_resamples: BOOTSTRAP_RESAMPLES,
        order_blocks: ORDER_BLOCKS,
        rounds_per_block: 2,
        resampling_units: RESAMPLING_UNITS,
        workloads,
    };
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, &report).expect("the report serializes");
    writeln!(output).expect("the report newline writes");
}
