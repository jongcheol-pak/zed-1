use std::{
    cell::RefCell,
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use windows::Win32::Graphics::{
    Direct3D11::{ID3D11Device, ID3D11DeviceContext},
    DirectComposition::{IDCompositionDevice, IDCompositionVisual},
};

/// The DirectComposition objects behind a window, for stacking visuals above its swap chain.
#[derive(Clone)]
pub struct CompositionHandles {
    /// The device that owns the window's visual tree. Commit through it after changing visuals.
    pub device: IDCompositionDevice,
    /// The root visual, whose content is the window's swap chain. Child visuals are composed
    /// above it.
    pub root: IDCompositionVisual,
    /// The Direct3D device the composition device was created from, for filling surfaces.
    pub d3d11_device: ID3D11Device,
    /// The immediate context of `d3d11_device`.
    pub d3d11_context: ID3D11DeviceContext,
    /// Changes whenever the objects are recreated (for example after a device loss); visuals
    /// built on an older generation belong to a dead device and must be rebuilt.
    pub generation: u64,
}

/// Returns the composition objects of the window `hwnd`, or `None` if the window has none
/// (DirectComposition disabled, not created yet, or closed). Main thread only.
pub fn composition_for_hwnd(hwnd: isize) -> Option<CompositionHandles> {
    HANDLES.with_borrow(|table| table.get(hwnd).cloned())
}

pub(crate) fn register_composition(hwnd: isize, handles: impl FnOnce(u64) -> CompositionHandles) {
    HANDLES.with_borrow_mut(|table| table.insert(hwnd, handles(next_generation())));
}

pub(crate) fn forget_composition(hwnd: isize, generation: u64) {
    HANDLES.with_borrow_mut(|table| table.remove(hwnd, generation));
}

fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    static HANDLES: RefCell<HandleTable<CompositionHandles>> = RefCell::new(HandleTable::default());
}

trait Generation {
    fn generation(&self) -> u64;
}

impl Generation for CompositionHandles {
    fn generation(&self) -> u64 {
        self.generation
    }
}

/// Handles by window. Removal names the generation it drops, so dropping the objects of a
/// device that was already replaced does not forget the new ones.
struct HandleTable<T> {
    entries: HashMap<isize, T>,
}

impl<T> Default for HandleTable<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<T: Generation> HandleTable<T> {
    fn insert(&mut self, hwnd: isize, handles: T) {
        self.entries.insert(hwnd, handles);
    }

    fn get(&self, hwnd: isize) -> Option<&T> {
        self.entries.get(&hwnd)
    }

    fn remove(&mut self, hwnd: isize, generation: u64) {
        if self
            .entries
            .get(&hwnd)
            .is_some_and(|handles| handles.generation() == generation)
        {
            self.entries.remove(&hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Generation for u64 {
        fn generation(&self) -> u64 {
            *self
        }
    }

    #[test]
    fn a_rebuild_bumps_the_generation() {
        let mut table = HandleTable::default();
        let first = next_generation();
        table.insert(1, first);
        let second = next_generation();
        table.insert(1, second);
        assert_ne!(first, second);
        assert_eq!(table.get(1), Some(&second));
        // The replaced objects are dropped after the new ones are registered.
        table.remove(1, first);
        assert_eq!(table.get(1), Some(&second));
    }

    #[test]
    fn a_closed_window_is_forgotten() {
        let mut table = HandleTable::default();
        let generation = next_generation();
        table.insert(1, generation);
        table.insert(2, next_generation());
        table.remove(1, generation);
        assert_eq!(table.get(1), None);
        assert!(table.get(2).is_some());
    }
}
