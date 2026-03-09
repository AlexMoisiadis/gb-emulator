use std::sync::OnceLock;
use std::sync::atomic::{ AtomicU64, Ordering };

#[derive(Clone, Copy, Debug)]
pub enum Category {
    CpuIrq,
    CpuCtrl,
    CpuHalt,
    MmuIo,
    GpuStat,
    GpuLcdc,
    Frame,
    Harness,
}

impl Category {
    #[inline]
    const fn bit(self) -> u32 {
        match self {
            Category::CpuIrq => 1 << 0,
            Category::CpuCtrl => 1 << 1,
            Category::CpuHalt => 1 << 2,
            Category::MmuIo => 1 << 3,
            Category::GpuStat => 1 << 4,
            Category::GpuLcdc => 1 << 5,
            Category::Frame => 1 << 6,
            Category::Harness => 1 << 7,
        }
    }
}

static TRACE_STEP: AtomicU64 = AtomicU64::new(0);
static STRUCTURED_ENABLED: OnceLock<bool> = OnceLock::new();
static FILTER_MASK: OnceLock<u32> = OnceLock::new();

const DEFAULT_MASK: u32 =
    Category::CpuIrq.bit() |
    Category::CpuHalt.bit() |
    Category::GpuStat.bit() |
    Category::Frame.bit() |
    Category::Harness.bit();

#[inline]
pub fn set_step(step: u64) {
    TRACE_STEP.store(step, Ordering::Relaxed);
}

#[inline]
pub fn step() -> u64 {
    TRACE_STEP.load(Ordering::Relaxed)
}

#[inline]
pub fn structured_enabled() -> bool {
    *STRUCTURED_ENABLED.get_or_init(
        || std::env::var("GB_TRACE_STRUCTURED").ok().as_deref() == Some("1")
    )
}

#[inline]
pub fn enabled(category: Category) -> bool {
    if !structured_enabled() {
        return false;
    }
    let mask = *FILTER_MASK.get_or_init(parse_mask_from_env);
    (mask & category.bit()) != 0
}

fn parse_mask_from_env() -> u32 {
    let raw = match std::env::var("GB_TRACE_FILTER") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            return DEFAULT_MASK;
        }
    };

    let mut mask = 0u32;
    for part in raw.split(',') {
        let key = part.trim().to_ascii_lowercase();
        match key.as_str() {
            "cpu_irq" => {
                mask |= Category::CpuIrq.bit();
            }
            "cpu_ctrl" => {
                mask |= Category::CpuCtrl.bit();
            }
            "cpu_halt" => {
                mask |= Category::CpuHalt.bit();
            }
            "mmu_io" => {
                mask |= Category::MmuIo.bit();
            }
            "gpu_stat" => {
                mask |= Category::GpuStat.bit();
            }
            "gpu_lcdc" => {
                mask |= Category::GpuLcdc.bit();
            }
            "frame" => {
                mask |= Category::Frame.bit();
            }
            "harness" => {
                mask |= Category::Harness.bit();
            }
            _ => {}
        }
    }

    if mask == 0 {
        DEFAULT_MASK
    } else {
        mask
    }
}
