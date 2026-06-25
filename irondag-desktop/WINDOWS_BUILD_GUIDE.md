# Windows Build Guide - IronDAG Desktop

**Complete step-by-step guide for compiling the IronDAG Desktop application on Windows**

---

## Prerequisites Installation

### 1. Install Visual Studio Build Tools 2022

**Required for Rust MSVC toolchain and Tauri compilation**

1. Download: [Visual Studio Build Tools 2022](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022)
2. Run the installer
3. Select **"Desktop development with C++"**
4. Ensure these components are checked:
   - MSVC v143 - VS 2022 C++ x64/x86 build tools
   - Windows 10 SDK (10.0.19041.0 or later)
   - C++ CMake tools for Windows
5. Click **Install** (requires ~6GB disk space)
6. Restart your computer after installation

### 2. Install Node.js

**Required for React frontend and npm package management**

1. Download: [Node.js 20.x LTS](https://nodejs.org/en/download/)
2. Run the installer (use default settings)
3. Verify installation:
   ```powershell
   node --version  # Should show v20.x.x
   npm --version   # Should show 10.x.x
   ```

### 3. Install Rust

**Required for Tauri backend**

1. Download: [rustup-init.exe](https://rustup.rs/)
2. Run the installer
3. Choose option **1** (Proceed with installation - default)
4. Restart your terminal/PowerShell
5. Verify installation:
   ```powershell
   rustc --version  # Should show rustc 1.75.0 or later
   cargo --version  # Should show cargo 1.75.0 or later
   ```

---

## Building the Desktop App

### Step 1: Clone the Repository

```powershell
# Clone the IronDAG repository
git clone https://github.com/RunTimeAdmin/IronDAG.git
cd irondag/irondag-desktop
```

### Step 2: Install Dependencies

```powershell
# Install Node.js dependencies
npm install
```

**Expected output:**
```
added 423 packages, and audited 424 packages in 45s
```

### Step 3: Build the Application

```powershell
# Build the desktop application (Release mode)
npm run tauri build
```

**Build time:** 3-5 minutes (first build may take longer due to Rust compilation)

**Expected output:**
```
   Compiling irondag-desktop v0.2.0
    Finished `release` profile [optimized] target(s) in 3m 24s
    Bundling IronDAG-Desktop_0.2.0_x64_en-US.msi
```

### Step 4: Locate the Installer

**MSI Installer:**
```
irondag-desktop\src-tauri\target\release\bundle\msi\IronDAG-Desktop_0.2.0_x64_en-US.msi
```

**Portable EXE:**
```
irondag-desktop\src-tauri\target\release\irondag-desktop.exe
```

---

## Installation

### Option 1: MSI Installer (Recommended)

1. Double-click `IronDAG-Desktop_0.2.0_x64_en-US.msi`
2. Click **Next** through the installation wizard
3. Choose installation location (default: `C:\Program Files\IronDAG Desktop`)
4. Click **Install**
5. Desktop shortcut will be created automatically

### Option 2: Portable EXE

1. Copy `irondag-desktop.exe` to any folder
2. Run directly (no installation required)
3. Create shortcut manually if desired

---

## Running the Application

### Step 1: Start the IronDAG Node

**Open PowerShell:**
```powershell
cd path\to\irondag\irondag-blockchain
cargo run --release --bin node
```

**Wait for:**
```
✅ JSON-RPC server listening on http://127.0.0.1:8546
✅ Node initialization complete - ready to accept RPC requests
```

### Step 2: Launch the Desktop App

**From Start Menu:**
- Search for "IronDAG Desktop"
- Click to launch

**Or from Command Line:**
```powershell
& "C:\Program Files\IronDAG Desktop\IronDAG Desktop.exe"
```

---

## Troubleshooting

### Build Errors

#### Error: "link.exe not found"

**Problem:** Visual Studio Build Tools not installed or not in PATH

**Solution:**
```powershell
# Verify Visual Studio installation
"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\*\bin\Hostx64\x64\link.exe"
```

If not found, reinstall Visual Studio Build Tools with C++ components.

#### Error: "failed to run custom build command for `tauri-build`"

**Problem:** Missing Windows SDK

**Solution:**
1. Open Visual Studio Installer
2. Click **Modify** on Build Tools 2022
3. Ensure "Windows 10/11 SDK" is checked
4. Click **Modify** to install

#### Error: "EPERM: operation not permitted"

**Problem:** File locked by another process

**Solution:**
```powershell
# Stop all Node/Cargo processes
Get-Process | Where-Object {$_.Name -like "*node*" -or $_.Name -like "*cargo*"} | Stop-Process -Force

# Clean and rebuild
npm run tauri build
```

### Connection Issues

#### "Failed to fetch status"

**Problem:** Desktop app cannot connect to node

**Solution:**
1. Verify node is running:
   ```powershell
   Test-NetConnection -ComputerName 127.0.0.1 -Port 8545
   ```
   Should return: `TcpTestSucceeded: True`

2. Test RPC manually:
   ```powershell
   $body = @{jsonrpc='2.0'; method='eth_blockNumber'; params=@(); id=1} | ConvertTo-Json
   Invoke-WebRequest -Uri 'http://127.0.0.1:8546' -Method POST -ContentType 'application/json' -Body $body
   ```
   Should return: `{"jsonrpc":"2.0","result":"0x...","id":1}`

3. Check Windows Firewall:
   - Allow incoming connections on port 8546
   - Add exception for `node.exe`

#### "Connection timeout" or "UI freezes"

**Problem:** Outdated version with lock contention issues

**Solution:** Update to Phase 2 (v0.2.0+):
```powershell
cd path\to\irondag
git pull origin master
cd irondag-blockchain
cargo clean
cargo build --release --bin node
```

---

## Development Mode (For Testing)

**Terminal 1 - Start Node:**
```powershell
cd irondag-blockchain
cargo run --release --bin node
```

**Terminal 2 - Start Desktop App (Dev Mode):**
```powershell
cd irondag-desktop
npm run tauri dev
```

**Benefits:**
- Hot reload for frontend changes
- DevTools accessible (Ctrl+Shift+I)
- Faster iteration for development

---

## System Requirements

**Minimum:**
- Windows 10 (64-bit, version 1909 or later)
- Intel/AMD x64 processor
- 4GB RAM
- 500MB disk space
- Internet connection

**Recommended:**
- Windows 11 (64-bit)
- Intel/AMD x64 processor (4+ cores)
- 8GB RAM
- 1GB disk space (for blockchain data)
- Broadband internet connection

---

## Version Information

**Current Version:** 0.2.0 (Phase 2 Complete)

**What's New in 0.2.0:**
- ✅ Zero RPC timeout issues (fixed lock contention)
- ✅ Responsive UI during mining
- ✅ <100ms RPC response time (was 30+ seconds)
- ✅ Concurrent operations (mining + RPC + UI)
- ✅ Production tested with 1700+ blocks

**Previous Version:** 0.1.0 (Initial Release)

---

## Build Artifacts

After successful build, you'll have:

1. **MSI Installer** (Signed, Installable)
   - Location: `src-tauri\target\release\bundle\msi\IronDAG-Desktop_0.2.0_x64_en-US.msi`
   - Size: ~15MB
   - Includes: Application + Dependencies

2. **Portable EXE** (No Installation)
   - Location: `src-tauri\target\release\irondag-desktop.exe`
   - Size: ~12MB
   - Requires: No installation, run directly

3. **Debug Symbols** (For Development)
   - Location: `src-tauri\target\release\irondag-desktop.pdb`
   - Size: ~3MB

---

## Security Notes

**Code Signing:**
- Current build is **unsigned** (development version)
- Windows SmartScreen may show warning
- For production, obtain code signing certificate

**Antivirus:**
- Some antivirus may flag unsigned executables
- Add exception for `irondag-desktop.exe` if needed
- Source code is open and auditable

---

## Support

**Issues:** https://github.com/RunTimeAdmin/IronDAG/issues  
**Documentation:** https://irondag.io/docs  
**Community:** https://discord.gg/irondag  

---

**Built on:** January 14, 2026  
**Build System:** Tauri 2.0 + React 19 + Rust 1.75+  
**Target:** Windows 10/11 x64
