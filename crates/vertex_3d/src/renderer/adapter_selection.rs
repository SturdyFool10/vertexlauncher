//! Adapter enumeration and selection helpers for explicit GPU control.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::AdapterPreference;

/// Explicit adapter selection strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterSelector {
    Preference(AdapterPreference),
    Slot(usize),
    Hashed(u64),
}

impl Default for AdapterSelector {
    fn default() -> Self {
        Self::Preference(AdapterPreference::Default)
    }
}

/// Serializable snapshot of one enumerated GPU adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableAdapter {
    pub slot: usize,
    pub name: String,
    pub backend: wgpu::Backend,
    pub device_type: wgpu::DeviceType,
    pub vendor: u32,
    pub device: u32,
    pub driver: String,
    pub driver_info: String,
    pub features_hash: u64,
    pub limits_hash: u64,
    pub selection_hash: u64,
    pub surface_supported: bool,
}

impl AvailableAdapter {
    pub fn from_adapter(
        adapter: &wgpu::Adapter,
        slot: usize,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Self {
        let info = adapter.get_info();
        let features_hash = stable_hash(&format!("{:?}", adapter.features()));
        let limits = adapter.limits();
        let limits_hash = stable_hash(&(
            limits.max_texture_dimension_1d,
            limits.max_texture_dimension_2d,
            limits.max_texture_dimension_3d,
            limits.max_bind_groups,
            limits.max_dynamic_uniform_buffers_per_pipeline_layout,
            limits.max_dynamic_storage_buffers_per_pipeline_layout,
            limits.max_storage_buffers_per_shader_stage,
            limits.max_uniform_buffers_per_shader_stage,
            limits.max_sampled_textures_per_shader_stage,
            limits.max_samplers_per_shader_stage,
        ));
        let surface_supported = surface.is_none_or(|surface| adapter.is_surface_supported(surface));

        let selection_hash = stable_hash(&(
            slot,
            &info.name,
            format!("{:?}", info.backend),
            format!("{:?}", info.device_type),
            info.vendor,
            info.device,
            &info.driver,
            &info.driver_info,
            features_hash,
            limits_hash,
        ));

        Self {
            slot,
            name: info.name,
            backend: info.backend,
            device_type: info.device_type,
            vendor: info.vendor,
            device: info.device,
            driver: info.driver,
            driver_info: info.driver_info,
            features_hash,
            limits_hash,
            selection_hash,
            surface_supported,
        }
    }
}

pub async fn enumerate_adapters(
    instance: &wgpu::Instance,
    backends: wgpu::Backends,
    surface: Option<&wgpu::Surface<'_>>,
) -> Vec<AvailableAdapter> {
    let adapters = instance.enumerate_adapters(backends).await;
    describe_adapter_slice(&adapters, surface)
}

pub fn describe_adapter_slice(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
) -> Vec<AvailableAdapter> {
    adapters
        .iter()
        .enumerate()
        .map(|(slot, adapter)| AvailableAdapter::from_adapter(adapter, slot, surface))
        .collect()
}

pub fn select_adapter_from_slice(
    adapters: &[wgpu::Adapter],
    surface: Option<&wgpu::Surface<'_>>,
    selector: AdapterSelector,
) -> Option<wgpu::Adapter> {
    let described = describe_adapter_slice(adapters, surface);
    let slot = select_adapter_slot(&described, selector)?;
    adapters.get(slot).cloned()
}

pub fn select_adapter_slot(
    adapters: &[AvailableAdapter],
    selector: AdapterSelector,
) -> Option<usize> {
    match selector {
        AdapterSelector::Preference(preference) => select_by_preference(adapters, preference),
        AdapterSelector::Slot(slot) => adapters
            .iter()
            .find(|adapter| adapter.slot == slot && adapter.surface_supported)
            .map(|adapter| adapter.slot),
        AdapterSelector::Hashed(hash) => adapters
            .iter()
            .find(|adapter| adapter.selection_hash == hash && adapter.surface_supported)
            .map(|adapter| adapter.slot),
    }
}

fn select_by_preference(
    adapters: &[AvailableAdapter],
    preference: AdapterPreference,
) -> Option<usize> {
    adapters
        .iter()
        .filter(|adapter| adapter.surface_supported)
        .filter_map(|adapter| {
            adapter_preference_score(adapter, preference).map(|score| (score, adapter.slot))
        })
        .max_by_key(|(score, slot)| (*score, std::cmp::Reverse(*slot)))
        .map(|(_, slot)| slot)
}

fn adapter_preference_score(
    adapter: &AvailableAdapter,
    preference: AdapterPreference,
) -> Option<i32> {
    let base = match adapter.device_type {
        wgpu::DeviceType::DiscreteGpu => 400,
        wgpu::DeviceType::IntegratedGpu => 300,
        wgpu::DeviceType::VirtualGpu => 200,
        wgpu::DeviceType::Other => 100,
        wgpu::DeviceType::Cpu => 0,
    };

    let score = match preference {
        AdapterPreference::Default | AdapterPreference::HighPerformance => {
            match adapter.device_type {
                wgpu::DeviceType::DiscreteGpu => 1000 + base,
                wgpu::DeviceType::IntegratedGpu => 800 + base,
                wgpu::DeviceType::VirtualGpu => 600 + base,
                wgpu::DeviceType::Other => 400 + base,
                wgpu::DeviceType::Cpu => return None,
            }
        }
        AdapterPreference::LowPower => match adapter.device_type {
            wgpu::DeviceType::IntegratedGpu => 1000 + base,
            wgpu::DeviceType::DiscreteGpu => 800 + base,
            wgpu::DeviceType::VirtualGpu => 600 + base,
            wgpu::DeviceType::Other => 400 + base,
            wgpu::DeviceType::Cpu => return None,
        },
        AdapterPreference::DiscreteOnly => {
            if adapter.device_type != wgpu::DeviceType::DiscreteGpu {
                return None;
            }
            1000 + base
        }
        AdapterPreference::IntegratedOnly => {
            if adapter.device_type != wgpu::DeviceType::IntegratedGpu {
                return None;
            }
            1000 + base
        }
    };
    Some(score)
}

fn stable_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(
        slot: usize,
        device_type: wgpu::DeviceType,
        surface_supported: bool,
    ) -> AvailableAdapter {
        AvailableAdapter {
            slot,
            name: format!("gpu-{slot}"),
            backend: wgpu::Backend::Vulkan,
            device_type,
            vendor: 1,
            device: slot as u32,
            driver: String::new(),
            driver_info: String::new(),
            features_hash: 1,
            limits_hash: 1,
            selection_hash: stable_hash(&(slot, format!("{device_type:?}"))),
            surface_supported,
        }
    }

    #[test]
    fn low_power_prefers_integrated_when_available() {
        let adapters = vec![
            adapter(0, wgpu::DeviceType::DiscreteGpu, true),
            adapter(1, wgpu::DeviceType::IntegratedGpu, true),
        ];
        assert_eq!(
            select_adapter_slot(
                &adapters,
                AdapterSelector::Preference(AdapterPreference::LowPower)
            ),
            Some(1)
        );
    }

    #[test]
    fn strict_integrated_only_returns_none_without_integrated_gpu() {
        let adapters = vec![adapter(0, wgpu::DeviceType::DiscreteGpu, true)];
        assert_eq!(
            select_adapter_slot(
                &adapters,
                AdapterSelector::Preference(AdapterPreference::IntegratedOnly),
            ),
            None
        );
    }

    #[test]
    fn hashed_selection_uses_stable_runtime_hash() {
        let adapters = vec![adapter(0, wgpu::DeviceType::DiscreteGpu, true)];
        assert_eq!(
            select_adapter_slot(
                &adapters,
                AdapterSelector::Hashed(adapters[0].selection_hash)
            ),
            Some(0)
        );
    }
}
