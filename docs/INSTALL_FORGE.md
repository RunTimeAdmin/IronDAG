# Installing Foundry (forge) on Windows

Foundry gives you `forge`, `cast`, `anvil`, and `chisel`. Use any **one** of the options below.

---

## Option 1: Install with Cargo (recommended on Windows)

You already have Rust/Cargo at `C:\Users\Admin01\.cargo\bin\cargo.exe`. Use it to install Foundry:

1. **Open a terminal** (PowerShell or Command Prompt) where `cargo` is on PATH.

2. **Install all Foundry tools** (one command, may take several minutes):

   ```powershell
   cargo install --git https://github.com/foundry-rs/foundry --profile release --locked forge cast chisel anvil
   ```

3. Binaries are placed in `C:\Users\Admin01\.cargo\bin\` (same as your other Rust tools). If that folder is already on your PATH, you can run:

   ```powershell
   forge --version
   forge test
   ```

**Requirements:** Rust (rustup), and on Windows: **Visual Studio** with the **“Desktop development with C++”** workload (rustup usually prompts for this).

---

## Option 2: Git BASH or WSL (official installer)

The official installer is a shell script, so it needs a Unix-like shell on Windows.

1. Install **Git for Windows** (includes Git BASH) from https://git-scm.com/download/win, or use **WSL** (Windows Subsystem for Linux).

2. Open **Git BASH** or a **WSL terminal** and run:

   ```bash
   curl -L https://foundry.paradigm.xyz | bash
   ```

3. Restart the terminal (or run `source ~/.bashrc`).

4. Install Foundry:

   ```bash
   foundryup
   ```

5. Foundry is installed in `~/.foundry/bin`. Add that to your PATH in Git BASH/WSL, or call the tools by full path. To use `forge` from **PowerShell**, you’d need to add the Foundry bin path to your **Windows** PATH (e.g. `C:\Users\Admin01\.foundry\bin` if foundryup put it there under your home).

---

## Option 3: Precompiled binaries

1. Open **Releases**: https://github.com/foundry-rs/foundry/releases  
2. Download the **Windows** archive (e.g. `foundry_win32_amd64_*.zip` or similar).  
3. Extract it to a folder (e.g. `C:\foundry`).  
4. Add that folder (or the `bin` subfolder) to your **PATH**.

Then run:

```powershell
forge --version
```

---

## After installation: run Solidity tests

From the `irondag` repo root:

1. **Install the forge-std library** (needed for the tests in `test/`):

   ```powershell
   cd "c:\Users\Admin01\Desktop\MondoShawan Blockchain\irondag"
   forge install foundry-rs/forge-std
   ```

2. **Run tests**:

   ```powershell
   forge test
   ```

---

## Quick reference

| Method              | Command / link |
|---------------------|----------------|
| **Cargo (Windows)** | `cargo install --git https://github.com/foundry-rs/foundry --profile release --locked forge cast chisel anvil` |
| **Git BASH / WSL**  | `curl -L https://foundry.paradigm.xyz \| bash` then `foundryup` |
| **Precompiled**     | https://github.com/foundry-rs/foundry/releases |

Official docs: https://getfoundry.sh/introduction/installation
