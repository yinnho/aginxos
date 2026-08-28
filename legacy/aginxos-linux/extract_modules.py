#!/usr/bin/env python3
"""Extract kernel modules from Debian kernel package for initramfs."""
import sys, os

deb_path = sys.argv[1]
dest_dir = sys.argv[2]

# Modules to extract and their source paths within the deb
modules = [
    ('kernel/drivers/net/ethernet/intel/e1000/e1000.ko', 'e1000.ko'),
    ('kernel/net/core/failover.ko', 'failover.ko'),
    ('kernel/drivers/net/net_failover.ko', 'net_failover.ko'),
    ('kernel/drivers/net/virtio_net.ko', 'virtio_net.ko'),
    ('kernel/drivers/virtio/virtio_mmio.ko', 'virtio_mmio.ko'),
    ('kernel/drivers/virtio/virtio_pci.ko', 'virtio_pci.ko'),
    ('kernel/drivers/virtio/virtio_pci_modern_dev.ko', 'virtio_pci_modern_dev.ko'),
    ('kernel/drivers/virtio/virtio_pci_legacy_dev.ko', 'virtio_pci_legacy_dev.ko'),
]

with open(deb_path, 'rb') as f:
    data = f.read()

# Parse ar archive to find data.tar.xz
offset = 8  # skip '!<arch>\n'
while offset < len(data):
    hdr = data[offset:offset+60]
    name = hdr[0:16].decode('ascii', errors='replace').strip()
    size_str = hdr[48:58].decode('ascii', errors='replace').strip()
    if not size_str:
        break
    size = int(size_str)
    if 'data.tar' in name:
        tmp = '/tmp/kmod.tar.xz'
        with open(tmp, 'wb') as out:
            out.write(data[offset+60:offset+60+size])
        os.makedirs(dest_dir, exist_ok=True)

        # Build tar extract paths
        tar_paths = [f'./lib/modules/6.1.0-44-arm64/{src}' for src, _ in modules]
        extract_cmd = f'xz -dc {tmp} | tar xf - -C /tmp ' + ' '.join(tar_paths) + ' 2>/dev/null'
        os.system(extract_cmd)

        for src, dst in modules:
            full_src = f'/tmp/lib/modules/6.1.0-44-arm64/{src}'
            if os.path.exists(full_src):
                os.rename(full_src, os.path.join(dest_dir, dst))
                print(f'  Added {dst}')
            else:
                print(f'  WARNING: {dst} not found in package')

        os.system('rm -rf /tmp/lib /tmp/kmod.tar.xz')
        break
    offset = offset + 60 + size
    if offset % 2:
        offset += 1
