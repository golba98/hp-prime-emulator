//! Portable ARM926 architectural state for the production CPU owner. Uses
//! register APIs only: Unicorn's opaque context contains native pointers and
//! is used solely for same-process rollback, never written into an image.
use serde::{Deserialize, Serialize};
use unicorn_engine::{ArmCpuModel, RegisterARM as R, RegisterARMCP, Unicorn, uc_error};

const MODES: [u32; 6] = [0x10, 0x11, 0x12, 0x13, 0x17, 0x1b];
const GPRS: [R; 16] = [
    R::R0,
    R::R1,
    R::R2,
    R::R3,
    R::R4,
    R::R5,
    R::R6,
    R::R7,
    R::R8,
    R::R9,
    R::R10,
    R::R11,
    R::R12,
    R::SP,
    R::LR,
    R::PC,
];
// ARM926's state-bearing CP15 registers: control, translation table, domains,
// data/instruction fault status, fault address, D/I lockdown, FCSE/context ID.
// Cache/TLB maintenance operations and read-only IDs are never replayed writes.
const CP15: [(u32, u32); 10] = [
    (1, 0),
    (2, 0),
    (3, 0),
    (5, 0),
    (5, 1),
    (6, 0),
    (9, 0),
    (9, 1),
    (13, 0),
    (13, 1),
];
// FPINST is a deprecated no-op in this backend, not restorable state.
const FP_CONTROL: [R; 3] = [R::FPEXC, R::FPSCR, R::FPSID];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bank {
    pub mode: u32,
    pub r8_to_lr: [u32; 7],
    pub spsr: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coprocessor {
    pub secure: u32,
    pub crn: u32,
    pub opc2: u32,
    pub value: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmCpuImage {
    pub version: u32,
    pub runtime_version: u32,
    pub model: i32,
    pub gprs: [u32; 16],
    pub cpsr: u32,
    pub banks: Vec<Bank>,
    pub fp_data: [u64; 32],
    pub fp_control: [u32; 3],
    pub cp15: Vec<Coprocessor>,
}
fn runtime_version() -> u32 {
    // SAFETY: uc_version permits null outputs and has no engine/native state.
    unsafe { unicorn_engine::uc_version(std::ptr::null_mut(), std::ptr::null_mut()) }
}
fn cp(secure: u32, crn: u32, opc2: u32) -> RegisterARMCP {
    RegisterARMCP::new().cp(15).sec(secure).crn(crn).opc2(opc2)
}
fn coprocessors(cpu: &Unicorn<'_, ()>) -> Result<Vec<Coprocessor>, String> {
    let mut result = Vec::new();
    for secure in [0, 1] {
        for (crn, opc2) in CP15 {
            let mut register = cp(secure, crn, opc2);
            match cpu.reg_read_arm_coproc(&mut register) {
                Ok(()) => result.push(Coprocessor {
                    secure,
                    crn,
                    opc2,
                    value: register.val,
                }),
                // An absent secure bank is part of the pinned CPU's schema.
                Err(uc_error::ARG) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    if !result.iter().any(|r| r.crn == 1 && r.opc2 == 0) {
        return Err("ARM926 system-control register is unavailable".into());
    }
    Ok(result)
}
impl ArmCpuImage {
    fn read(cpu: &mut Unicorn<'_, ()>) -> Result<Self, String> {
        let model = cpu.ctl_get_cpu_model().map_err(|e| e.to_string())?;
        if model != ArmCpuModel::Model_926 as i32 {
            return Err("CPU image requires the production ARM926 model".into());
        }
        let read = |reg| {
            cpu.reg_read(reg)
                .map(|v| v as u32)
                .map_err(|e| e.to_string())
        };
        let cpsr = read(R::CPSR)?;
        let mut gprs = [0; 16];
        for (value, reg) in gprs.iter_mut().zip(GPRS) {
            *value = read(reg)?;
        }
        let mut fp_control = [0; 3];
        for (value, reg) in fp_control.iter_mut().zip(FP_CONTROL) {
            *value = read(reg)?;
        }
        let mut fp_data = [0; 32];
        for (index, value) in fp_data.iter_mut().enumerate() {
            *value = cpu
                .reg_read(R::D0 as i32 + index as i32)
                .map_err(|e| e.to_string())?;
        }
        let cp15 = coprocessors(cpu)?;
        let mut banks = Vec::new();
        for mode in MODES {
            cpu.reg_write(R::CPSR, u64::from((cpsr & !0x1f) | mode))
                .map_err(|e| e.to_string())?;
            let mut values = [0; 7];
            for (value, reg) in values.iter_mut().zip(GPRS[8..15].iter()) {
                *value = cpu.reg_read(*reg).map_err(|e| e.to_string())? as u32;
            }
            banks.push(Bank {
                mode,
                r8_to_lr: values,
                spsr: if mode == 0x10 {
                    None
                } else {
                    Some(cpu.reg_read(R::SPSR).map_err(|e| e.to_string())? as u32)
                },
            });
        }
        Ok(Self {
            version: 1,
            runtime_version: runtime_version(),
            model,
            gprs,
            cpsr,
            banks,
            fp_data,
            fp_control,
            cp15,
        })
    }
    pub fn capture(cpu: &mut Unicorn<'_, ()>) -> Result<Self, String> {
        let context = cpu.context_init().map_err(|e| e.to_string())?;
        let image = Self::read(cpu);
        cpu.context_restore(&context).map_err(|e| e.to_string())?;
        let image = image?;
        image.validate(cpu)?;
        Ok(image)
    }
    pub(crate) fn validate(&self, cpu: &Unicorn<'_, ()>) -> Result<(), String> {
        if self.version != 1
            || self.model != ArmCpuModel::Model_926 as i32
            || self.runtime_version != runtime_version()
            || cpu.ctl_get_cpu_model().map_err(|e| e.to_string())? != self.model
            || self.banks.len() != MODES.len()
        {
            return Err("CPU image version/runtime/model/bank set mismatch".into());
        }
        let mode = self.cpsr & 0x1f;
        if !MODES.contains(&mode) && mode != 0x1f {
            return Err("Invalid ARM926 active mode".into());
        }
        if self.cpsr & (1 << 24) != 0 {
            return Err("Jazelle execution is not supported by this CPU backend".into());
        }
        let alignment = if self.cpsr & 0x20 != 0 { 2 } else { 4 };
        if self.gprs[15] % alignment != 0 {
            return Err("CPU image PC disagrees with its instruction set".into());
        }
        for (bank, expected) in self.banks.iter().zip(MODES) {
            if bank.mode != expected || bank.spsr.is_some() != (expected != 0x10) {
                return Err("Invalid ARM926 bank or saved status register".into());
            }
            if expected != 0x11 && bank.r8_to_lr[..5] != self.banks[0].r8_to_lr[..5] {
                return Err("Non-FIQ register banks disagree".into());
            }
        }
        let active = self
            .banks
            .iter()
            .find(|b| b.mode == if mode == 0x1f { 0x10 } else { mode })
            .unwrap();
        if self.gprs[8..15] != active.r8_to_lr {
            return Err("Active registers disagree with the selected bank".into());
        }
        if self.fp_control[2] != cpu.reg_read(R::FPSID).map_err(|e| e.to_string())? as u32 {
            return Err("CPU floating-point identity mismatch".into());
        }
        let expected = coprocessors(cpu)?;
        if self.cp15.len() != expected.len()
            || self.cp15.iter().zip(expected).any(|(a, b)| {
                (a.secure, a.crn, a.opc2) != (b.secure, b.crn, b.opc2)
                    || a.value > u64::from(u32::MAX)
            })
        {
            return Err("CPU coprocessor schema mismatch".into());
        }
        Ok(())
    }
    pub fn restore(&self, cpu: &mut Unicorn<'_, ()>) -> Result<(), String> {
        self.validate(cpu)?;
        let rollback = cpu.context_init().map_err(|e| e.to_string())?;
        let result = self.write(cpu).and_then(|()| {
            let restored = Self::capture(cpu)?;
            if restored != *self {
                return Err("Restored CPU failed architectural readback".into());
            }
            Ok(())
        });
        if result.is_err() {
            cpu.context_restore(&rollback).map_err(|e| e.to_string())?;
        }
        result
    }
    fn write(&self, cpu: &mut Unicorn<'_, ()>) -> Result<(), String> {
        for bank in &self.banks {
            cpu.reg_write(R::CPSR, u64::from((self.cpsr & !0x1f) | bank.mode))
                .map_err(|e| e.to_string())?;
            for (reg, value) in GPRS[8..15].iter().zip(bank.r8_to_lr) {
                cpu.reg_write(*reg, u64::from(value))
                    .map_err(|e| e.to_string())?;
            }
            if let Some(spsr) = bank.spsr {
                cpu.reg_write(R::SPSR, u64::from(spsr))
                    .map_err(|e| e.to_string())?;
            }
        }
        for register in &self.cp15 {
            let mut value = cp(register.secure, register.crn, register.opc2);
            value.val = register.value;
            cpu.reg_write_arm_coproc(&value)
                .map_err(|e| e.to_string())?;
        }
        for (index, value) in self.fp_data.iter().enumerate() {
            cpu.reg_write(R::D0 as i32 + index as i32, *value)
                .map_err(|e| e.to_string())?;
        }
        for (reg, value) in FP_CONTROL.into_iter().zip(self.fp_control) {
            cpu.reg_write(reg, u64::from(value))
                .map_err(|e| e.to_string())?;
        }
        cpu.reg_write(R::CPSR, u64::from(self.cpsr))
            .map_err(|e| e.to_string())?;
        for (reg, value) in GPRS.into_iter().zip(self.gprs) {
            cpu.reg_write(reg, u64::from(value))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
