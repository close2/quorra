//! Which driver stack talks to the hardware is the host's to say.
//!
//! ADR 0017, from the caller's feedback §12: their project owner's Windows machine
//! crashed inside an Intel Vulkan driver, and nothing in this library could ask for the
//! DX12 one. `create_instance_with` is the whole answer, so what these tests hold is the
//! three claims that make it usable:
//!
//! - **A named backend set is the set the device is chosen from.** An instance
//!   restricted to Vulkan yields Vulkan adapters and no others, and the device built on
//!   it says so.
//! - **A set this machine cannot supply refuses, and names what it found.** Not a
//!   panic, not a hang, not a device on the backend the host was avoiding:
//!   `DeviceError::NoAdapter` with an empty `available`, which is the signature of this
//!   mistake rather than of a broken driver.
//! - **What a host offers its user is what its constructors can honour.**
//!   `adapter_names_on` answers for the instance it is given, so a `--backend`
//!   flag and an `--adapter` flag cannot contradict each other.
//!
//! Note what cannot be tested here: **no machine in this project runs Windows**, and
//! neither adapter on it is reachable through DX12. The mechanism is exercised with the
//! backends this machine does have — Vulkan against everything, and the empty set
//! against nothing — which is the same code path with a different mask.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, DeviceError, Options, wgpu};

/// The software adapter, as everywhere in this suite: deterministic, always present.
const SOFTWARE: &str = "llvmpipe";

/// An instance restricted to Vulkan sees Vulkan adapters, and the device built on it
/// runs on one.
#[test]
fn a_named_backend_is_the_backend_the_device_uses() {
    let instance = quorra_gpu::create_instance_with(wgpu::Backends::VULKAN);
    let names = Device::adapter_names_on(&instance);
    assert!(
        !names.is_empty(),
        "this machine has Vulkan adapters; the whole suite depends on it"
    );

    for name in names {
        let device = Device::headless_with_instance(
            &instance,
            &Options {
                adapter: Some(name.clone()),
                ..Options::default()
            },
        )
        .expect("an adapter this instance enumerated must yield a device");
        assert!(
            device.description().contains("Vulkan"),
            "an instance restricted to Vulkan produced '{}'",
            device.description()
        );
    }
}

/// Restricting the set drops adapters and invents none: what a Vulkan-only instance
/// sees is exactly what the unrestricted instance sees through Vulkan.
#[test]
fn restricting_the_set_only_removes() {
    let all = Device::adapter_names();
    let vulkan =
        Device::adapter_names_on(&quorra_gpu::create_instance_with(wgpu::Backends::VULKAN));
    for name in &vulkan {
        assert!(
            all.contains(name),
            "'{name}' appeared under a restriction and not without one"
        );
    }
    assert!(
        vulkan.len() <= all.len(),
        "a subset of the backends cannot enumerate more adapters"
    );
}

/// A backend set this machine cannot supply is not a panic and not a silent fallback
/// onto the backend the host was avoiding: it is the typed refusal, naming what was
/// there — nothing.
#[test]
fn a_set_this_machine_cannot_supply_refuses_by_name() {
    let instance = quorra_gpu::create_instance_with(wgpu::Backends::empty());
    assert!(
        Device::adapter_names_on(&instance).is_empty(),
        "an instance holding no backend can enumerate no adapter"
    );

    match Device::headless_with_instance(&instance, &Options::default()) {
        Err(DeviceError::NoAdapter {
            requested,
            available,
        }) => {
            assert!(requested.is_none(), "no filter was asked for");
            assert!(
                available.is_empty(),
                "the empty list is what says the instance, not the machine, was empty"
            );
        }
        Err(other) => panic!("the wrong refusal: {other}"),
        Ok(device) => panic!(
            "an instance with no backend produced a device on '{}'",
            device.description()
        ),
    }
}

/// The same refusal with an adapter filter, because the two are separate arms of
/// `select_adapter` and only one of them was covered above.
#[test]
fn a_filter_against_no_backend_refuses_too() {
    let instance = quorra_gpu::create_instance_with(wgpu::Backends::empty());
    match Device::headless_with_instance(
        &instance,
        &Options {
            adapter: Some(SOFTWARE.into()),
            ..Options::default()
        },
    ) {
        Err(DeviceError::NoAdapter {
            requested,
            available,
        }) => {
            assert_eq!(requested.as_deref(), Some(SOFTWARE));
            assert!(available.is_empty());
        }
        Err(other) => panic!("the wrong refusal: {other}"),
        Ok(device) => panic!(
            "an instance with no backend produced a device on '{}'",
            device.description()
        ),
    }
}

/// `create_instance()` keeps its meaning: every backend, and the adapters this suite's
/// other tests rely on. The parameter is an addition, not a change of default.
#[test]
fn the_unrestricted_entry_point_is_unchanged() {
    let default = Device::adapter_names_on(&quorra_gpu::create_instance());
    let explicit =
        Device::adapter_names_on(&quorra_gpu::create_instance_with(wgpu::Backends::all()));
    assert_eq!(
        default, explicit,
        "create_instance() is create_instance_with(Backends::all())"
    );
    assert!(
        default.iter().any(|name| name.contains(SOFTWARE)),
        "the software adapter this suite runs on must still be found: {default:?}"
    );
}
