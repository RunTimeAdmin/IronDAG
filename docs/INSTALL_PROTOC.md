# Installing protoc (Protocol Buffers compiler) on Windows

The node build compiles gRPC `.proto` files and requires the **protoc** binary. If you see:

```text
Could not find `protoc`. ... Try setting the `PROTOC` environment variable
```

use one of the options below.

---

## Option 1: Download prebuilt binary (recommended)

1. **Download** the Windows zip from:  
   https://github.com/protocolbuffers/protobuf/releases  
   Look for `protoc-<version>-win64.zip` (e.g. `protoc-28.2-win64.zip`).

2. **Extract** the zip (e.g. to `C:\protoc`).  
   You should have a folder containing `bin\protoc.exe` and `include\`.

3. **Add to PATH** (one of these):
   - **Temporary (current PowerShell):**
     ```powershell
     $env:PATH = "C:\protoc\bin;" + $env:PATH
     ```
   - **Permanent:**  
     Settings → System → About → Advanced system settings → Environment Variables → edit **Path** → New → `C:\protoc\bin` (use your actual path).

4. **Verify:**
   ```powershell
   protoc --version
   ```

5. **Rebuild the node:**
   ```powershell
   cd irondag\irondag-blockchain
   cargo build --release --bin node
   ```

---

## Option 2: Set PROTOC only (no PATH change)

If you prefer not to add protoc to PATH:

1. Download and extract as in Option 1.
2. Set the environment variable (PowerShell, current session):
   ```powershell
   $env:PROTOC = "C:\protoc\bin\protoc.exe"
   ```
3. Run `cargo build --release --bin node` in the same session.

---

## Option 3: Package managers

- **Chocolatey:** `choco install protoc`
- **Scoop:** `scoop install protobuf`
- **winget:** `winget install Google.Protobuf` (if available)

Then run `protoc --version` and rebuild the node.
