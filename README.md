# emulator3ds

`emulator3ds` is a **production-quality emulator core framework** for building a Nintendo 3DS emulator in Rust, designed for native and WebAssembly targets.

## Implemented milestone set

- **ARM11 ISA + system/coprocessor subset**
  - Data processing: `AND`, `EOR`, `SUB`, `RSB`, `ADD`, `ADC`, `SBC`, `TST`, `TEQ`, `CMP`, `CMN`, `ORR`, `MOV`, `BIC`, `MVN`
  - Control flow: `B`, `BL`, `BX`
  - Memory transfer: `LDR`/`STR` (immediate, pre/post index + writeback subset)
  - Multiply: `MUL`/`MLA` subset
  - System: `MRS`/`MSR` subset, `SWI`, `WFI`
  - Coprocessor: CP15 `MRC`/`MCR` register-bank subset
- **Exception model with SPSR banking and return semantics**
  - Undefined and software-interrupt vectors
  - Mode switches to UND/SVC
  - SPSR capture per exception mode
  - Exception return via `MOVS pc, lr` CPSR restore path
- **PICA200 command/shader pipeline scaffold**
  - GPU command queue (`Clear`, `DrawPoint`)
  - Shader-constant transform stage
  - Deterministic command consumption per tick
- **Kernel/service emulation scaffold**
  - SWI -> service dispatch (`Yield`, `GetTick`, unknown passthrough)
  - Service call logging + introspection
- **Timing and A/V sync model**
  - Cycle-based timing model
  - Derived audio/video pacing and desync signal
- **Filesystem/title-content loading pipeline**
  - `3DST` title package parser
  - Content table handling and ROM extraction/loading

## Build and test

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --target wasm32-unknown-unknown
```
