# Installation & Updates

## Installation

You can install lazydns using a number of methods depending on your platform and preferences.

### 1. Install via `cargo install`, the rusty way
Installs the latest published crate to your Cargo bin directory:
```bash
cargo install lazydns
```

### 2.1 Debian / Ubuntu (.deb via APT repo)
Add the repository key and source, then install with `apt`:
```bash
sudo curl -fsSL https://raw.githubusercontent.com/lazywalker/apt/refs/heads/master/debian/key.asc -o /etc/apt/trusted.gpg.d/lazywalker.asc

echo "deb https://raw.githubusercontent.com/lazywalker/apt/refs/heads/master/debian/ stable main" | sudo tee /etc/apt/sources.list.d/lazywalker.list

sudo apt update
sudo apt install lazydns
```

### 2.2 Raspberry Pi OS (Trixie, arm64)
Use the same repo but restrict to `arm64` architecture in the sources.list entry:
```bash
sudo curl -fsSL https://raw.githubusercontent.com/lazywalker/apt/refs/heads/master/debian/key.asc -o /etc/apt/trusted.gpg.d/lazywalker.asc

echo "deb [arch=arm64] https://raw.githubusercontent.com/lazywalker/apt/refs/heads/master/debian/ stable main" | sudo tee /etc/apt/sources.list.d/lazywalker.list

sudo apt update
sudo apt install lazydns
```

### 2.3 Install on Arch Linux
You can install lazydns from the Arch User Repository (AUR) using an AUR helper like `yay`:
```bash
yay -S lazydns-bin
```

### 3. Systemd Service Setup (via apt & systemd Linux)
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
- From APT: `sudo apt update && sudo apt upgrade` (package upgrades coming from the repo)
- From Docker: pull the new image and recreate the container:
```bash
docker pull lazywalker/lazydns:latest
docker rm -f lazydns
docker run ... (recreate with same args)
```

## Notes
- If you build from source and plan to package, use the `cargo-deb` or native packaging tooling appropriate for your distribution.
- For cross-compilation and reproducible builds, consult the `scripts/cross_build.sh` helper and the `docker/` folder for example Dockerfiles.
