use crate::config::AppConfig;
use crate::core::hardware::HardwareAdapter;
use tracing::warn;

#[cfg(target_os = "linux")]
pub mod linux;
mod simulated;

#[cfg(target_os = "linux")]
pub use linux::{LinuxAdapter, LinuxAdapterConfig};
pub use simulated::{SimulatedAdapter, Simulator};

pub fn get_adapter(config: &AppConfig) -> Box<dyn HardwareAdapter> {
    if config.simulation {
        let (adapter, controller) = simulated::SimulatedAdapter::new();

        std::thread::spawn(move || {
            for line in std::io::stdin().lines().map_while(Result::ok) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                match parts.as_slice() {
                    ["add", uuid] => controller.add_device(uuid, 64),
                    ["add"] => controller.add_device("123", 64),
                    ["rm", uuid] => controller.remove_device(uuid),
                    ["rm"] => controller.remove_device("123"),
                    _ => warn!(input = %line, "Invalid command. Use: 'add <uuid>' or 'rm <uuid>'"),
                }
            }
        });

        return Box::new(adapter);
    }

    #[cfg(target_os = "linux")]
    {
        let adapter_config = LinuxAdapterConfig {
            mount_base: config.mount_base.clone(),
            auto_mount: true,
        };
        Box::new(linux::LinuxAdapter::new(adapter_config))
    }

    #[cfg(not(target_os = "linux"))]
    {
        panic!("Non-simulation mode only supported on Linux");
    }
}
