//! What bring-up costs, one step at a time — the §7 measurement, attributable.
//!
//! The caller put GPU bring-up on its time-to-first-page (its ADR 0179: page one goes
//! to the graphics device, with no CPU first frame), which makes every number here
//! part of a launch a person waits through. Its feedback §8.1 then showed that the
//! single `adapter_enumeration` figure quorra used to report measured three unrelated
//! steps at once — the driver loader, surface creation and physical-device
//! enumeration — so a regression in it could not be attributed. This example prints
//! what replaced that number.
//!
//! **One configuration per process, and that is the whole design of this file.** A
//! second `wgpu::Instance` in the same process finds the driver loader warm and
//! reports a fraction of the true cost; the caller's own first bring-up harness made
//! exactly that mistake and read 4.4 ms for work that costs 26 ms. So this example
//! measures *one* device and exits, and a sweep is a shell loop:
//!
//! ```sh
//! for a in RADV llvmpipe; do cargo run --release -p quorra-gpu --example startup -- "$a"; done
//! cargo run --release -p quorra-gpu --example startup -- RADV hoisted
//! ```
//!
//! `hoisted` builds the device from an instance created first, the way a host would
//! on a thread at `main`'s first line — the lever of feedback §8.2. It reports no
//! instance number, because that step was not the constructor's; what it shows is the
//! blocking remainder, which is what a launch actually waits for once the instance is
//! made in parallel with reading the document.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::Duration;

use quorra_gpu::{Device, Options, StartupTimings};

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// One line per step, in the order the steps happen.
fn report(what: &str, description: &str, startup: &StartupTimings) {
    println!("{what} on {description}");
    match startup.instance_creation {
        Some(instance) => println!("  instance creation   {:8.3} ms", milliseconds(instance)),
        None => println!("  instance creation        —      (supplied by the caller)"),
    }
    println!(
        "  surface creation    {:8.3} ms",
        milliseconds(startup.surface_creation)
    );
    println!(
        "  adapter selection   {:8.3} ms",
        milliseconds(startup.adapter_selection)
    );
    println!(
        "  device creation     {:8.3} ms",
        milliseconds(startup.device_creation)
    );
    println!(
        "  = blocking total    {:8.3} ms",
        milliseconds(startup.blocking_total())
    );
    println!(
        "  pipelines (thread)  {:8.3} ms   — nothing blocks on this",
        startup.pipeline_compilation.map_or(f64::NAN, milliseconds)
    );
}

fn main() {
    // `hoisted` anywhere among the arguments; the other one, if any, is the adapter
    // filter. No adapter filter means `request_adapter` chooses — a different code
    // path with a different cost, and the one a caller taking `Options::default`
    // pays, so it must be measurable without inventing a name to filter on.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hoisted = args.iter().any(|arg| arg == "hoisted");
    let adapter = args.into_iter().find(|arg| arg != "hoisted");
    let options = Options {
        adapter: adapter.clone(),
        ..Options::default()
    };
    let what = match (&adapter, hoisted) {
        (Some(name), true) => format!("hoisted instance, adapter '{name}'"),
        (Some(name), false) => format!("Device::headless, adapter '{name}'"),
        (None, true) => "hoisted instance, wgpu's preferred adapter".to_owned(),
        (None, false) => "Device::headless, wgpu's preferred adapter".to_owned(),
    };

    // The one instance this process creates, either way.
    let device = if hoisted {
        let instance = quorra_gpu::create_instance();
        Device::headless_with_instance(&instance, &options)
    } else {
        Device::headless(&options)
    };
    let device = match device {
        Ok(device) => device,
        Err(error) => {
            eprintln!("no device: {error}");
            std::process::exit(1);
        }
    };
    // The warm set is compiled on a thread the constructor never waits for; waiting
    // here is what makes its duration readable, and it is charged to nothing.
    device.wait_until_warm();
    report(&what, device.description(), &device.startup());
}
