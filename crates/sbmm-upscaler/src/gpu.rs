//! What the machine's graphics card can actually run.
//!
//! Installing a DLL the hardware cannot load is worse than doing nothing: the
//! game silently loses the feature and the cause is invisible. So the adapter
//! is identified first and the offer is narrowed to what it supports.
//!
//! Only [`adapters`] touches the operating system. The classification below it
//! works on the adapter description string, which is why it can be tested with
//! the real names cards report rather than only on a machine that has one.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

impl Vendor {
    /// PCI vendor ids, which are what DXGI reports.
    pub fn from_pci_id(id: u32) -> Vendor {
        match id {
            0x10DE => Vendor::Nvidia,
            0x1002 | 0x1022 => Vendor::Amd,
            0x8086 => Vendor::Intel,
            _ => Vendor::Other,
        }
    }
}

/// A feature an upscaler DLL needs from the hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    /// Anything that runs the game at all.
    Universal,
    /// Tensor cores: RTX 20 series and later.
    DlssSuperResolution,
    /// The optical flow accelerator: RTX 40 series and later.
    DlssFrameGeneration,
}

/// One display adapter, as reported by the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Adapter {
    /// What the driver calls it, e.g. "NVIDIA GeForce RTX 4070 Ti".
    pub name: String,
    pub vendor: Vendor,
    /// Video memory in bytes, used only to pick the real card over a
    /// software or remote-desktop adapter.
    pub memory_bytes: u64,
}

impl Adapter {
    pub fn new(name: impl Into<String>, vendor: Vendor, memory_bytes: u64) -> Self {
        Self {
            name: name.into(),
            vendor,
            memory_bytes,
        }
    }

    /// Whether this card can make use of a DLL needing `capability`.
    pub fn supports(&self, capability: Capability) -> bool {
        match capability {
            Capability::Universal => true,
            Capability::DlssSuperResolution => {
                matches!(self.vendor, Vendor::Nvidia) && self.rtx_generation().is_some()
            }
            Capability::DlssFrameGeneration => {
                matches!(self.vendor, Vendor::Nvidia)
                    // Ada (40) introduced it; every later series keeps it.
                    && self.rtx_generation().is_some_and(|series| series >= 40)
            }
        }
    }

    /// The RTX series number — 20, 30, 40, 50 — or nothing for a card with no
    /// tensor cores.
    ///
    /// GTX cards, Quadro/Tesla parts and anything unrecognised come back as
    /// `None`, which is the safe answer: the offer is then withheld rather
    /// than made on a guess.
    pub fn rtx_generation(&self) -> Option<u16> {
        if self.vendor != Vendor::Nvidia {
            return None;
        }
        let name = self.name.to_ascii_uppercase();

        // Workstation cards carry the architecture in the name instead of a
        // series number, so they are matched by architecture directly.
        for (marker, series) in [
            ("RTX A", 30u16),
            ("RTX 6000 ADA", 40),
            ("RTX 5000 ADA", 40),
            ("RTX 4500 ADA", 40),
            ("RTX 4000 ADA", 40),
        ] {
            if name.contains(marker) {
                return Some(series);
            }
        }

        // Otherwise the model number follows "RTX": 2080, 3060 Ti, 4070, 5090.
        let after = name.split("RTX").nth(1)?.trim_start();
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        // Four digits, because "RTX 20" alone says nothing about the model.
        if digits.len() != 4 {
            return None;
        }
        let model: u16 = digits.parse().ok()?;
        // 4070 is a 40-series card: the leading digit is the generation.
        let series = model / 1000 * 10;
        // Only series that exist; a typo or a future name is not assumed.
        (20..=50).contains(&series).then_some(series)
    }
}

/// Every display adapter the system reports, best first.
#[cfg(windows)]
pub fn adapters() -> Vec<Adapter> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, DXGI_ADAPTER_FLAG,
        DXGI_ADAPTER_FLAG_SOFTWARE,
    };

    let mut found = Vec::new();
    // SAFETY: DXGI enumeration; every pointer comes from the factory and is
    // released when the returned COM object drops.
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return found;
        };
        let mut index = 0;
        while let Ok(adapter) = factory.EnumAdapters1(index) {
            index += 1;
            let adapter: IDXGIAdapter1 = adapter;
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            // The Basic Render Driver is always present and never the card
            // the game runs on.
            if DXGI_ADAPTER_FLAG(desc.Flags as i32) == DXGI_ADAPTER_FLAG_SOFTWARE {
                continue;
            }
            let name = String::from_utf16_lossy(
                &desc.Description[..desc
                    .Description
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(desc.Description.len())],
            );
            found.push(Adapter::new(
                name,
                Vendor::from_pci_id(desc.VendorId),
                desc.DedicatedVideoMemory as u64,
            ));
        }
    }

    // A laptop reports the integrated chip too; the one with the most video
    // memory is the one the game will be using.
    found.sort_by_key(|adapter| std::cmp::Reverse(adapter.memory_bytes));
    found
}

#[cfg(not(windows))]
pub fn adapters() -> Vec<Adapter> {
    // The manager only ships for Windows; this keeps the crate building, and
    // the UI treats an empty list as "hardware unknown".
    Vec::new()
}

/// The adapter the game is most likely running on.
pub fn primary_adapter() -> Option<Adapter> {
    adapters().into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nvidia(name: &str) -> Adapter {
        Adapter::new(name, Vendor::Nvidia, 8 << 30)
    }

    #[test]
    fn rtx_series_is_read_from_the_model_number() {
        assert_eq!(nvidia("NVIDIA GeForce RTX 2060").rtx_generation(), Some(20));
        assert_eq!(
            nvidia("NVIDIA GeForce RTX 3060 Ti").rtx_generation(),
            Some(30)
        );
        assert_eq!(
            nvidia("NVIDIA GeForce RTX 4070 Ti SUPER").rtx_generation(),
            Some(40)
        );
        assert_eq!(nvidia("NVIDIA GeForce RTX 5090").rtx_generation(), Some(50));
    }

    #[test]
    fn cards_without_tensor_cores_are_not_guessed_at() {
        assert_eq!(nvidia("NVIDIA GeForce GTX 1080 Ti").rtx_generation(), None);
        assert_eq!(
            nvidia("NVIDIA GeForce GTX 1660 SUPER").rtx_generation(),
            None
        );
        assert_eq!(
            Adapter::new("AMD Radeon RX 7900 XTX", Vendor::Amd, 24 << 30).rtx_generation(),
            None
        );
        assert_eq!(
            Adapter::new("Intel(R) Arc(TM) A770", Vendor::Intel, 16 << 30).rtx_generation(),
            None
        );
    }

    #[test]
    fn workstation_cards_are_placed_by_architecture() {
        assert_eq!(nvidia("NVIDIA RTX A4000").rtx_generation(), Some(30));
        assert_eq!(
            nvidia("NVIDIA RTX 4000 Ada Generation").rtx_generation(),
            Some(40)
        );
    }

    #[test]
    fn frame_generation_needs_a_40_series_but_super_resolution_does_not() {
        let ampere = nvidia("NVIDIA GeForce RTX 3080");
        assert!(ampere.supports(Capability::DlssSuperResolution));
        assert!(!ampere.supports(Capability::DlssFrameGeneration));

        let ada = nvidia("NVIDIA GeForce RTX 4060");
        assert!(ada.supports(Capability::DlssSuperResolution));
        assert!(ada.supports(Capability::DlssFrameGeneration));

        let blackwell = nvidia("NVIDIA GeForce RTX 5070");
        assert!(blackwell.supports(Capability::DlssFrameGeneration));
    }

    #[test]
    fn dlss_is_never_offered_to_a_card_that_cannot_run_it() {
        let pascal = nvidia("NVIDIA GeForce GTX 1070");
        assert!(!pascal.supports(Capability::DlssSuperResolution));
        assert!(!pascal.supports(Capability::DlssFrameGeneration));

        let radeon = Adapter::new("AMD Radeon RX 6800 XT", Vendor::Amd, 16 << 30);
        assert!(!radeon.supports(Capability::DlssSuperResolution));
        // But FSR runs on anything, including NVIDIA.
        assert!(radeon.supports(Capability::Universal));
        assert!(pascal.supports(Capability::Universal));
    }

    #[test]
    fn vendors_come_from_the_pci_id() {
        assert_eq!(Vendor::from_pci_id(0x10DE), Vendor::Nvidia);
        assert_eq!(Vendor::from_pci_id(0x1002), Vendor::Amd);
        assert_eq!(Vendor::from_pci_id(0x8086), Vendor::Intel);
        assert_eq!(Vendor::from_pci_id(0x1234), Vendor::Other);
    }
}
