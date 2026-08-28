#!/bin/bash
# 创建 boot.img for Pixel 5
# 
# boot.img 格式:
# - Android boot image header
# - kernel + ramdisk + dtb

KERNEL=$1
OUTPUT=$2

KERNEL_SIZE=$(stat -f%z "$KERNEL")

# Pixel 5 boot.img 参数
PAGE_SIZE=4096
KERNEL_OFFSET=0
RAMDISK_OFFSET=0  
DTB_OFFSET=1  

# 创建 boot.img (仅包含内核)
# 需要添加 boot image header
