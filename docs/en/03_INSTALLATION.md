# Installation & Updates

## Installation

You can install lazydns using a number of methods depending on your platform and preferences.

### 1. Install via `cargo install`, the rusty way
Installs the latest published crate to your Cargo bin directory:
```bash
cargo install lazydns
```

> **Note on the WebUI**: `cargo install` builds the server core only. For a binary with the WebUI bundled in, use the pre-built release binaries.

### 2.1 Debian / Ubuntu (amd64)
Download the `.deb` from [GitHub Releases](https://github.com/lazywalker/lazydns/releases) and install with `dpkg`:
```bash
curl -LO https://github.com/lazywalker/lazydns/releases/latest/download/lazydns_<version>-1_amd64.deb
sudo dpkg -i lazydns_<version>-1_amd64.deb
```

### 2.2 Raspberry Pi OS / arm64
Download the `arm64` `.deb` and install with `dpkg`:
```bash
curl -LO https://github.com/lazywalker/lazydns/releases/latest/download/lazydns_<version>-1_arm64.deb
sudo dpkg -i lazydns_<version>-1_arm64.deb
```

### 2.3 Install on Arch Linux
You can install lazydns from the Arch User Repository (AUR) using an AUR helper like `yay`:
```bash
yay -S lazydns-bin
```

### 3. Systemd Service Setup (Debian / Ubuntu / Raspberry Pi OS)
after installation, modify the config file at `/etc/lazydns/lazydns.yaml` as needed, then start the service:
```bash
sudo systemctl start lazydns
```
the service will auto-start on boot. Check status with:
```bash
sudo systemctl status lazydns
```
check logs with:
```bash
sudo journalctl -u lazydns -f
```
or view the log file at `/var/log/lazydns/lazydns.log.*`.

### 4. Homebrew (macOS / Linuxbrew)
Tap the Homebrew repository and install via `brew`:
```bash
brew tap lazywalker/tap
brew install lazydns

# make modifications to config file if needed
# then start the service
brew services start lazydns
```

### 5. Docker
Run lazydns from the official Docker image. Example command (adjust volumes, ports and environment as needed):
```bash
docker run -d \
	--name lazydns \
	-p 53:53/udp -p 53:53/tcp \
	-p 853:853/tcp -p 443:443/tcp \
	-p 784:784/tcp -p 8000:8000/tcp -p 8001:8001/tcp \
	-e TZ=Asia/Shanghai \
	-v /path/to/config:/etc/lazydns \
	lazywalker/lazydns:latest
```

## Upgrading
- From `cargo install`: `cargo install --force lazydns`
- From `.deb`: download the new `.deb` and run `sudo dpkg -i` again
- From Docker: pull the new image and recreate the container:
```bash
docker pull lazywalker/lazydns:latest
docker rm -f lazydns
docker run ... (recreate with same args)
```

## Notes
- If you build from source and plan to package, use the `cargo-deb` or native packaging tooling appropriate for your distribution.
- For cross-compilation and reproducible builds, consult the `scripts/cross_build.sh` helper and the `docker/` folder for example Dockerfiles.
