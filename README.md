# drop

A fast, lightweight CLI tool to transfer files between any devices on your local network using QR codes and direct HTTP links. No client app or installation is required on the receiving end.

## Installation

### 1. Pre-compiled Binaries (Recommended)
Download the latest compiled binary for your operating system from the [Releases](https://github.com/xxeisenberg/drop/releases) page. 

Rename the downloaded file to `drop`, make it executable, and move it into your PATH. For example, on Linux:
```bash
chmod +x drop
sudo mv drop /usr/local/bin/
```

### 2. From Source
If you have the Rust toolchain installed, you can build and install `drop` directly from source:
```bash
git clone https://github.com/xxeisenberg/drop.git
cd drop
cargo build --release
```
The executable can found at `./target/release/drop`

**OR**

```bash
cargo install --git https://github.com/xxeisenberg/drop.git
```
_(Make sure `$HOME/.cargo/bin` on Linux/macOS or `%USERPROFILE%\.cargo\bin` on Windows is in your system's PATH.)_
## Usage

### Send a file

```bash
drop send path/to/your/file.ext
```
The terminal will display a QR code and a local IP link. Scan the QR code with a phone, or open the link on any other computer on your network.

### Receive a file
Navigate to the folder where you want to save incoming files and run:

```bash
drop receive
```
Scan the QR code or open the provided link on another device. Select your file(s) and hit upload. It will save to your current working directory.

### Options
By default, `drop` uses port `1844`. You can specify a custom port (1024–65535) using the `-p` or `--port` flag:
```bash
drop receive --port 8080
```

## Network Requirements

* Both devices **must** be connected to the exact same Wi-Fi or local network.
* Ensure your host system's firewall allows incoming TCP traffic on the port you are using. 
  * *Example (ufw):* `sudo ufw allow 1844/tcp`
  * *Example (firewalld):* `sudo firewall-cmd --add-port=1844/tcp`
