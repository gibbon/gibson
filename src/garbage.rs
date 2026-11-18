// An easter egg after the film: search for "garbage" and a hidden file's tower rises
// out of the floor. Its contents are made up here (nothing from the film): fake
// low-level code skimming fractions of cents, and a hex dump of noise.

use crate::city::Rng;
use crate::fs_scan::Entry;
use std::path::PathBuf;

pub const NAME: &str = "GARBAGE";

/// Not a real file: nothing on disk is ever at this path.
pub fn path() -> PathBuf {
    PathBuf::from("\u{0}gibson-hidden\u{0}garbage")
}

pub fn entry() -> Entry {
    let mut rng = Rng(0x6A7B_A6E5);
    let ops = ["MOV", "XOR", "JMP", "CALL", "PUSH", "POP", "CMP", "JNZ", "SHR", "ADD", "LEA", "INT"];
    let regs = ["AX", "BX", "CX", "DX", "SI", "DI", "BP", "SP"];
    let notes = [
        "; skim residue of every transfer",
        "; round down, keep the remainder",
        "; remainder -> account 0x7731",
        "; hide in the nightly batch",
        "; erase own trace from the log",
        "; replicate to the next node",
        "; wake on the fifth cycle",
        "; do not touch: still running",
    ];
    let mut lines: Vec<(String, bool)> = vec![
        ("HIDDEN SEGMENT - NOT IN ANY DIRECTORY".into(), false),
        ("ORIGIN UNKNOWN   OWNER UNKNOWN".into(), false),
    ];
    for i in 0..420u32 {
        let line = if i % 9 == 4 {
            notes[rng.next() as usize % notes.len()].to_string()
        } else {
            format!(
                "{:04X}:{:04X}  {:<5}{}, {}",
                0x7F00 + (i / 16),
                (i * 3) & 0xFFFF,
                ops[rng.next() as usize % ops.len()],
                regs[rng.next() as usize % regs.len()],
                if rng.f32() < 0.5 { regs[rng.next() as usize % regs.len()].to_string() } else { format!("0x{:04X}", rng.next() as u16) }
            )
        };
        lines.push((line, false));
    }
    let head: Vec<u8> = (0..4096).map(|_| rng.next() as u8).collect();
    let meta = vec![
        ("TYPE".to_string(), "UNKNOWN - NO SIGNATURE".to_string()),
        ("BYTES".into(), "6,660,000".into()),
        ("MODIFIED".into(), "-".into()),
        ("CREATED".into(), "-".into()),
        ("ATTRIB".into(), "HIDDEN, SYSTEM, NOT IN ANY INDEX".into()),
        ("ENTROPY".into(), "7.99 BITS/BYTE  COMPRESSED / ENCRYPTED".into()),
        ("HEAD".into(), head.iter().take(8).map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")),
    ];
    Entry {
        name: NAME.into(),
        path: path(),
        is_dir: false,
        size: 6_660_000,
        child_count: 0,
        capped: false,
        modified: None,
        lines,
        head,
        types: Vec::new(),
        meta,
    }
}
