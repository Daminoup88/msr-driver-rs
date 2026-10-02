//! Demonstrates how WattSeal reads CPU package power using `msr-driver-rs` + PawnIO.
//!
//! Supports both Intel and AMD RAPL.
//!
//! Run (requires PawnIO already installed):
//! ```
//! cargo run --example wattseal_like
//! ```

#![cfg(windows)]

use std::{
    thread,
    time::{Duration, Instant},
};

use msr_driver_rs::{CpuVendor, MsrDriver};

// ---- Intel RAPL MSRs ----
const MSR_RAPL_POWER_UNIT: u32 = 0x0000_0606;
const MSR_PKG_ENERGY_STATUS: u32 = 0x0000_0611;

// ---- AMD Zen RAPL MSRs ----
const AMD_MSR_PWR_UNIT: u32 = 0xC001_0299;
const AMD_MSR_PKG_ENERGY: u32 = 0xC001_029B;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Handle --install flag for convenience
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--install") {
        println!("Installing PawnIO driver (requires Administrator)...");
        MsrDriver::install()?;
        println!("PawnIO driver installed and started successfully.");
        return Ok(());
    }

    // --- Driver installation check ---
    match MsrDriver::is_installed() {
        Ok(false) => {
            eprintln!("PawnIO driver is not installed.");
            eprintln!("Run the following once from an elevated (Administrator) terminal:");
            eprintln!("  cargo run --example wattseal_like -- --install");
            return Ok(());
        }
        Ok(true) => {}
        Err(e) => {
            eprintln!("Error checking driver status: {e}");
            return Err(e.into());
        }
    }

    // --- Open driver ---
    let driver = MsrDriver::new()?;

    // --- Detect vendor & read energy unit ---
    let vendor = CpuVendor::detect()?;
    println!("CPU vendor: {vendor:?}");

    match vendor {
        CpuVendor::Intel => run_intel_loop(&driver),
        CpuVendor::Amd => run_amd_loop(&driver),
    }
}

/// Intel RAPL power monitor loop.
/// Uses MSR_PKG_ENERGY_STATUS (0x611) with 32-bit wrap-around semantics.
fn run_intel_loop(driver: &MsrDriver) -> Result<(), Box<dyn std::error::Error>> {
    let unit_raw = driver.read_msr(MSR_RAPL_POWER_UNIT, 0)?;
    let energy_unit_exp = ((unit_raw >> 8) & 0x1F) as u32;
    let joules_per_lsb = 1.0f64 / (1u64 << energy_unit_exp) as f64;
    println!(
        "Intel RAPL energy unit: 2^-{energy_unit_exp} J = {:.6e} J/LSB",
        joules_per_lsb
    );

    let mut prev_energy = (driver.read_msr(MSR_PKG_ENERGY_STATUS, 0)? & 0xFFFF_FFFF) as u32;
    let mut prev_time = Instant::now();

    loop {
        thread::sleep(Duration::from_secs(1));

        let now = Instant::now();
        let cur_energy = (driver.read_msr(MSR_PKG_ENERGY_STATUS, 0)? & 0xFFFF_FFFF) as u32;
        let delta_raw = cur_energy.wrapping_sub(prev_energy) as f64;
        let elapsed_s = now.duration_since(prev_time).as_secs_f64();

        if elapsed_s > 0.0 {
            let delta_joules = delta_raw * joules_per_lsb;
            let power_watts = delta_joules / elapsed_s;
            println!("[Intel] pkg_power={power_watts:.3} W  delta_j={delta_joules:.6}  elapsed={elapsed_s:.3} s");
        }

        prev_energy = cur_energy;
        prev_time = now;
    }
}

/// AMD Zen 1–5 RAPL power monitor loop.
/// Uses MSR_PKG_ENERGY_STAT (0xC001_029B); AMD uses 64-bit counters with saturation.
fn run_amd_loop(driver: &MsrDriver) -> Result<(), Box<dyn std::error::Error>> {
    let unit_raw = driver.read_msr(AMD_MSR_PWR_UNIT, 0)?;
    let energy_unit_exp = ((unit_raw as u32) >> 8) & 0x1F;
    let joules_per_lsb = 1.0f64 / (1u64 << energy_unit_exp) as f64;
    println!(
        "AMD RAPL energy unit: 2^-{energy_unit_exp} J = {:.6e} J/LSB",
        joules_per_lsb
    );

    let mut prev_energy = driver.read_msr(AMD_MSR_PKG_ENERGY, 0)?;
    let mut prev_time = Instant::now();

    loop {
        thread::sleep(Duration::from_secs(1));

        let now = Instant::now();
        let cur_energy = driver.read_msr(AMD_MSR_PKG_ENERGY, 0)?;
        // AMD counters are 64-bit and monotonically increasing (saturating).
        let delta_raw = cur_energy.saturating_sub(prev_energy) as f64;
        let elapsed_s = now.duration_since(prev_time).as_secs_f64();

        if elapsed_s > 0.0 {
            let delta_joules = delta_raw * joules_per_lsb;
            let power_watts = delta_joules / elapsed_s;
            println!("[AMD] pkg_power={power_watts:.3} W  delta_j={delta_joules:.6}  elapsed={elapsed_s:.3} s");
        }

        prev_energy = cur_energy;
        prev_time = now;
    }
}
