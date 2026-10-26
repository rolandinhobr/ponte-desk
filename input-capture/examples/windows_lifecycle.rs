//! Native hook lifecycle smoke test. No clients, barriers or injected input.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use input_capture::{Backend, InputCapture};
    use std::time::{Duration, Instant};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
    fn handles() -> u32 {
        let mut count = 0;
        unsafe {
            GetProcessHandleCount(GetCurrentProcess(), &mut count).unwrap();
        }
        count
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    // Warm lazy Tokio/Win32 resources before checking for per-cycle leaks.
    rt.block_on(async {
        let mut capture = InputCapture::new(Some(Backend::Windows)).await?;
        capture.release().await?;
        drop(capture);
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;
    let before = handles();
    let mut samples = Vec::new();
    let mut max = Duration::ZERO;
    for cycle in 0..20 {
        let start = Instant::now();
        rt.block_on(async {
            let mut capture = InputCapture::new(Some(Backend::Windows)).await?;
            capture.release().await?;
            capture.terminate().await?;
            drop(capture);
            Ok::<(), Box<dyn std::error::Error>>(())
        })?;
        max = max.max(start.elapsed());
        samples.push(handles());
        println!("cycle {} ok, handles {}", cycle + 1, handles());
    }
    let after = handles();
    assert!(
        after <= before + 2,
        "native handles leaked: {before} -> {after}"
    );
    println!("handle samples: {samples:?}");
    println!(
        "{{\"passed\":true,\"real_hook_cycles\":20,\"clients\":0,\"injected_input\":false,\"handles_before\":{before},\"handles_after\":{after},\"max_cycle_ms\":{}}}",
        max.as_millis()
    );
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    println!("Windows-only native lifecycle smoke test");
}
