# WezTerm Remote Mux Connection Report

## Overview

WezTerm supports connecting to remote multiplexer instances via SSH and TLS connections. This report documents how the initial package deployment works, customization options, and available configurations.

## Connection Types

WezTerm supports three types of remote mux connections:
1. **SSH Domains** - Connect via SSH, run wezterm proxy on remote
2. **TLS Domains** - Direct TLS connection (optionally bootstrapped via SSH)
3. **Unix Domains** - Local socket connections (used internally)

---

## SSH Domain Connection Flow

### Initial Package Push and Installation

**WezTerm does NOT automatically push or install binaries to the remote system.** Instead, it **assumes wezterm is already installed** on the remote host and accessible in the PATH.

#### Connection Process (from `wezterm-client/src/client.rs`):

```rust
fn ssh_connect(
    &mut self,
    ssh_dom: SshDomain,
    initial: bool,
    ui: &mut ConnectionUI,
) -> anyhow::Result<()>
```

1. **SSH Connection Establishment**
   - Uses `wezterm_ssh` library to establish SSH connection
   - Leverages user's SSH config (`~/.ssh/config`)
   - Supports SSH key-based authentication

2. **Remote Command Execution**
   - **Initial connection**: Executes `wezterm cli --prefer-mux proxy` on remote
   - **Reconnection**: Executes `wezterm cli --prefer-mux --no-auto-start proxy` on remote
   - The `proxy` command acts as a stdio proxy to the remote mux server

3. **Auto-Start Behavior**
   - The `--prefer-mux` flag tells wezterm to connect to a mux server (vs GUI)
   - If no mux server is running, the proxy command will **automatically start one** (unless `--no-auto-start` is used)
   - The remote wezterm mux server starts as: `wezterm-mux-server --daemonize`

4. **Stream Setup**
   - stdin/stdout of the remote proxy command becomes the communication channel
   - Protocol data units (PDUs) are exchanged over this channel

#### Key Code Location
```
wezterm-client/src/client.rs:677-728
```

---

## Customization Options

### 1. Custom Remote Binary Path

**Configuration**: `remote_wezterm_path`

**Purpose**: Specify the path to wezterm on the remote system

**Example**:
```lua
config.ssh_domains = {
  {
    name = 'my.server',
    remote_address = '192.168.1.1',
    remote_wezterm_path = '/home/user/bin/wezterm',
  },
}
```

**Default Behavior**: If not specified, assumes `wezterm` is in PATH

**Code Reference**:
```rust
// wezterm-client/src/client.rs:673-675
fn wezterm_bin_path(path: &Option<String>) -> String {
    path.as_deref().unwrap_or("wezterm").to_string()
}
```

### 2. Complete Proxy Command Override

**Configuration**: `override_proxy_command`

**Purpose**: Completely replace the default proxy invocation

**Example**:
```lua
config.ssh_domains = {
  {
    name = 'my.server',
    remote_address = '192.168.1.1',
    override_proxy_command = '/custom/path/to/wezterm cli proxy',
  },
}
```

**Use Cases**:
- Custom wrapper scripts
- Environment setup before starting proxy
- Alternative proxy implementations

**Code Reference**:
```rust
// wezterm-client/src/client.rs:688-694
let cmd = if let Some(cmd) = ssh_dom.override_proxy_command.clone() {
    cmd
} else if initial {
    format!("{} cli --prefer-mux proxy", proxy_bin)
} else {
    format!("{} cli --prefer-mux --no-auto-start proxy", proxy_bin)
};
```

---

## Available SSH Domain Configurations

### Complete Configuration Structure

From `config/src/ssh.rs`:

```rust
pub struct SshDomain {
    /// Domain name (must be unique)
    pub name: String,
    
    /// Host:port to connect to
    pub remote_address: String,
    
    /// Disable SSH agent authentication
    pub no_agent_auth: bool,
    
    /// Username for SSH authentication
    pub username: Option<String>,
    
    /// Auto-connect at startup
    pub connect_automatically: bool,
    
    /// Connection timeout
    pub timeout: Duration,
    
    /// Latency threshold for local echo (ms)
    pub local_echo_threshold_ms: Option<u64>,
    
    /// Show lag indicator overlay (deprecated)
    pub overlay_lag_indicator: bool,
    
    /// Path to wezterm binary on remote
    pub remote_wezterm_path: Option<String>,
    
    /// Override the entire proxy command
    pub override_proxy_command: Option<String>,
    
    /// SSH backend (Ssh2 or LibSsh)
    pub ssh_backend: Option<SshBackend>,
    
    /// Multiplexing mode
    pub multiplexing: SshMultiplexing,
    
    /// SSH config option overrides
    pub ssh_option: HashMap<String, String>,
    
    /// Default program to run
    pub default_prog: Option<Vec<String>>,
    
    /// Assume remote shell type
    pub assume_shell: Shell,
}
```

### Configuration Examples

#### Basic SSH Multiplexing Domain
```lua
config.ssh_domains = {
  {
    name = 'my.server',
    remote_address = '192.168.1.1',
    username = 'myuser',
    connect_automatically = true,
  },
}
```

#### Custom Binary Location
```lua
config.ssh_domains = {
  {
    name = 'my.server',
    remote_address = '192.168.1.1',
    remote_wezterm_path = '/opt/wezterm/bin/wezterm',
  },
}
```

#### SSH Config Overrides
```lua
config.ssh_domains = {
  {
    name = 'my.server',
    remote_address = '192.168.1.1',
    ssh_option = {
      identityfile = '/path/to/id_rsa',
      port = '2222',
    },
  },
}
```

#### Plain SSH (No Multiplexing)
```lua
config.ssh_domains = {
  {
    name = 'my.server',
    remote_address = '192.168.1.1',
    multiplexing = 'None',  -- No mux, direct SSH only
    assume_shell = 'Posix',  -- Enable shell integration
  },
}
```

#### Local Echo for High Latency Connections
```lua
config.ssh_domains = {
  {
    name = 'high-latency.server',
    remote_address = '203.0.113.1',
    -- Enable predictive local echo if latency > 10ms
    local_echo_threshold_ms = 10,
  },
}
```

---

## Configuration Details

### Multiplexing Modes

**`SshMultiplexing::WezTerm`** (default)
- Requires wezterm on remote
- Full multiplexing support
- Persistent sessions
- Reconnectable

**`SshMultiplexing::None`**
- Plain SSH connection (like `wezterm ssh`)
- No wezterm required on remote
- Session ends when connection drops
- Useful for WSL or simple connections

### Shell Assumptions

**`Shell::Unknown`** (default)
- No assumptions about remote shell
- Limited integration features

**`Shell::Posix`**
- Assumes POSIX-compliant shell (bash, zsh, etc.)
- Enables proper CWD tracking
- Allows spawning tabs in same directory
- Uses syntax: `env -C DIR $SHELL`

### SSH Backends

**`SshBackend::LibSsh`** (default)
- Native libssh implementation
- Better performance
- More features

**`SshBackend::Ssh2`**
- Legacy ssh2 library
- Fallback option

---

## TLS Domain Bootstrap via SSH

WezTerm also supports TLS domains that can be bootstrapped via SSH:

### Configuration (from `config/src/tls.rs`)

```lua
config.tls_clients = {
  {
    name = 'my.tls.server',
    remote_address = '192.168.1.1:8080',
    
    -- Bootstrap via SSH to get TLS certificates
    bootstrap_via_ssh = 'user@192.168.1.1:22',
    
    -- Path to wezterm on remote (for bootstrap)
    remote_wezterm_path = '/usr/local/bin/wezterm',
  },
}
```

### Bootstrap Process

1. Connects via SSH
2. Runs `wezterm cli tlscreds` on remote
3. Downloads TLS certificates
4. Saves certs locally
5. Establishes TLS connection directly

**Code Reference**: `wezterm-client/src/client.rs:859-925`

---

## Auto-Discovery of SSH Hosts

Since version `20230408-112425-69ae8472`, wezterm auto-discovers SSH hosts from `~/.ssh/config`:

```lua
-- Automatically populated domains:
-- SSH:hostname     - Plain SSH, no mux
-- SSHMUX:hostname  - SSH with multiplexing
```

### Customizing Auto-Discovery

```lua
config.ssh_domains = wezterm.default_ssh_domains()

-- Customize all auto-discovered domains
for _, dom in ipairs(config.ssh_domains) do
  dom.assume_shell = 'Posix'
  dom.remote_wezterm_path = '/usr/local/bin/wezterm'
end
```

---

## Important Notes

### Installation Requirements

**WezTerm MUST be pre-installed on the remote system** by the user through one of:
- Package manager (apt, yum, brew, etc.)
- Manual binary installation
- Building from source

The client **does not** and **cannot** push binaries to the remote.

### Version Compatibility

The client performs version checks:
```rust
// wezterm-client/src/client.rs:66-68
"Please install the same version of wezterm on both the client and server!\n\
 The server version is {} but the client version is {}\n",
```

### Proxy Command Mechanism

The proxy command (`wezterm cli proxy`) acts as a stdio bridge:
- Reads from stdin → forwards to mux server socket
- Reads from mux server socket → writes to stdout
- Auto-starts mux server if not running (unless `--no-auto-start`)

**Code Location**: `wezterm/src/cli/proxy.rs`

---

## Summary

### How Initial Connection Works
1. SSH connection established using user's SSH config
2. Remote wezterm binary is executed: `wezterm cli --prefer-mux proxy`
3. Proxy command checks for running mux server
4. If not running, starts: `wezterm-mux-server --daemonize`
5. Proxy bridges stdio to mux server's unix socket
6. Protocol messages exchanged over SSH channel

### Customization Points
1. **`remote_wezterm_path`** - Specify binary location
2. **`override_proxy_command`** - Complete command override
3. **`ssh_option`** - Override SSH config per-domain
4. **`multiplexing`** - Choose mux mode (WezTerm or None)
5. **`assume_shell`** - Enable shell integration features

### Key Configuration Options
- Connection: `remote_address`, `username`, `timeout`
- Authentication: `no_agent_auth`, `ssh_option`
- Behavior: `connect_automatically`, `multiplexing`
- Remote binary: `remote_wezterm_path`, `override_proxy_command`
- Performance: `local_echo_threshold_ms`
- Integration: `assume_shell`, `default_prog`

---

## File References

**Configuration Structs**:
- `config/src/ssh.rs` - SshDomain struct (lines 51-107)
- `config/src/tls.rs` - TlsDomainClient struct (lines 30-97)

**Connection Logic**:
- `wezterm-client/src/client.rs:677-728` - SSH connection
- `wezterm-client/src/client.rs:810-925` - TLS bootstrap
- `wezterm-client/src/domain.rs:931-982` - Attach logic

**Proxy Command**:
- `wezterm/src/cli/proxy.rs` - Proxy implementation
- `wezterm/src/cli/mod.rs:53-74` - CLI flags

**Documentation**:
- `docs/config/lua/SshDomain.md` - User documentation
- `docs/multiplexing.md` - General multiplexing docs

