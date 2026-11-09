# KT5 hardware report (G0)

Fill in from `scripts/probe-kindle.sh` output. **Do not** record serial
numbers, MAC/IP addresses, account names or personal paths. A `null` means
"not measured" — never guess.

```yaml
hardware:
  commercial_model: "Kindle Basic 11th Gen (2022)"
  codename: "Cava (to verify)"
  firmware: null            # /etc/version.txt, identifiers removed
  kernel: null              # uname -r
  uname_machine: null       # uname -m
  cpu_abi: null             # from cpuinfo + G1 CLI result
  mem_total_kib: null
  mem_available_idle_kib: null           # Amazon UI idle
  mem_available_koreader_kib: null       # while KOReader runs
  framebuffer_nodes: []
  framebuffer_geometry: "not probed yet" # /sys/class/graphics/fb0/virtual_size, bpp, stride
  input_devices: []                      # /proc/bus/input/devices names + handlers
  user_storage_mount_flags: null         # watch for noexec on /mnt/us
  usbnetwork_route: "not verified"
  ko_reader_return_path: "not verified"
  screen_owner_processes: []             # observed only (G3 §21.1)
```

## G0 acceptance

- [ ] Model and firmware recorded, without sensitive identifiers.
- [ ] `/proc/meminfo` collected in normal reading state and after leaving KOReader.
- [ ] Storage mount flags recorded (watch for `noexec`).
- [ ] A second independent method of regaining control exists.
- [ ] A written restoration plan exists before running graphics.
