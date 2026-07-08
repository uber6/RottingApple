mod credentials;
mod homekit;
mod legacy_pin;
mod prompt;
mod tlv;

pub use credentials::{PairingManager, format_pin};
// Experimental / incomplete HAP pairing — not used by PairingManager (see homekit.rs).
pub use homekit::{
    PairingSession as HapPairingSession, finish_pairing as finish_hap_pairing,
    pair_device as pair_hap_device, start_pairing as start_hap_pairing,
};
pub use legacy_pin::{LegacyPairingSession, finish_pairing, start_pairing};
pub use prompt::{prompt_pin_interactive, prompt_pin_interactive_async};
