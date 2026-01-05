use crate::core::hardware::HardwareAdapter;

#[cfg(target_os = "linux")]
mod linux;
mod simulated;

pub fn get_adapter(simulation: bool) -> Box<dyn HardwareAdapter> {
    if simulation {
        let (adapter, controller) = simulated::SimulatedAdapter::new();

        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lines() {
                if let Ok(cmd) = line {
                    let parts: Vec<&str> = cmd.trim().split_whitespace().collect();
                    match parts.get(0).copied() {
                        Some("add") => controller.add_device(parts.get(1).unwrap_or(&"123"), 64),
                        Some("rm") => controller.remove_device(parts.get(1).unwrap_or(&"123")),
                        _ => println!("(Simulator) Use: 'add <uuid>' or 'remove <uuid>'"),
                    }
                }
            }
        });

        return Box::new(adapter);
    }

    #[cfg(target_os = "linux")]
    {
        return Box::new(linux::LinuxAdapter);
    }
}
