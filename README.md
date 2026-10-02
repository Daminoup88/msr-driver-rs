# msr-driver-rs [![](https://img.shields.io/crates/v/msr-driver-rs.svg)](https://crates.io/crates/msr-driver-rs)

Minimal Rust wrapper around Windows MSR drivers (PawnIO, Scaphandre or WinRing0).

## Features

- `PawnIO` (default): Uses the PawnIO driver
- `scaphandre`: Uses the Scaphandre RAPL driver
- `winring0`: Uses the WinRing0 driver

**Note:** Only one feature can be enabled at a time.

### Using PawnIO

To use the default PawnIO driver, execute the following:

```bash
cargo add msr-driver-rs
```

### Using Scaphandre

To use the Scaphandre driver, execute the following:

```bash
cargo add msr-driver-rs --no-default-features --features scaphandre
```

### Using WinRing0

To use WinRing0 instead of Scaphandre, execute the following:

```bash
cargo add msr-driver-rs --no-default-features --features winring0
```

## Usage

```rust
use msr_driver_rs::MsrDriver;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Once, from an elevated process:
    // MsrDriver::install()?;

    let driver = MsrDriver::new()?;
    let energy_raw = driver.read_msr(0x0000_0611, 0)?; // Intel MSR_PKG_ENERGY_STATUS, AMD MSR_PKG_ENERGY is 0xC001_029B
    println!("energy raw: {energy_raw:#x}");
    Ok(())
}
```

## Ownership Rules

- `install()` uses **detect-first**: if PawnIO is already running (installed by the user or another tool), it just starts it without recreating the service.
- `uninstall()` only removes the service if **`msr-driver-rs` itself installed it** (tracked via a marker file). Third-party PawnIO installations are never touched.
- `legacy_winring0_cleanup()` safely removes any old `WinRing0_1_2_0` service previously deployed by this crate to `%TEMP%\msr-driver-rs\WinRing0x64.sys`.

## Admin operations

```rust
MsrDriver::install()?;   // Deploy PawnIO.sys + register + start (requires admin)
MsrDriver::uninstall()?; // Stop + delete service (only if we installed it)
```

## Example

```bash
cargo run --example wattseal_like # read power (PawnIO must be installed)
```
It polls the energy counter, calculates the joules consumed, and average power consumption, such as what [WattSeal](https://github.com/Daminoup88/WattSeal) does.

```bash
cargo run --example wattseal_like -- --install # install + start (requires admin)
```

## License

The crate source code is licensed under **GPL-3.0**.
