# drop

A fast, lightweight CLI tool to transfer files between any devices on your local network using QR codes, direct HTTP links, or automatic peer discovery. No client app or installation is required on the receiving end.

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

### Join an existing session
If another `drop` instance is running on your network, you can connect to it directly without scanning QR codes or entering links:

```bash
drop join                        # join a host in 'send' mode (download)
drop join path/to/file.ext       # join a host in 'receive' mode (upload)
```
`join` uses mDNS to automatically discover active hosts on the local network. If multiple hosts are found, you'll get an interactive menu to pick one.


### Options

| Flag | Description |
|------|-------------|
| `-p`, `--port <PORT>` | Custom port (1024–65535, default: `1844`) |
| `--max-size <MB>` | Maximum upload file size in megabytes (default: no limit) |
| `--encrypt` | Enable end-to-end encryption for CLI-to-CLI transfers |

```bash
drop send --encrypt ./secret.pdf
drop receive --port 8080 --encrypt
```

### Encryption

When `--encrypt` is passed, `drop` uses AES-256-GCM streaming encryption. Both the sender and receiver must use the `--encrypt` flag. The encryption key is exchanged automatically over mDNS when using `join`.

Browser-based uploads/downloads (via QR code or link) always use plaintext HTTP since the browser cannot participate in the key exchange.

## Network Requirements

* Both devices **must** be connected to the exact same Wi-Fi or local network.
* Ensure your host system's firewall allows incoming TCP traffic on the port you are using. 
  * *Example (ufw):* `sudo ufw allow 1844/tcp`
  * *Example (firewalld):* `sudo firewall-cmd --add-port=1844/tcp`
