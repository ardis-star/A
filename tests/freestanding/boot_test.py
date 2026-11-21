#!/usr/bin/env python3
"""Boot generated +A freestanding code, using a test-only Multiboot loader.

Requires a Linux/x86-64 host, GNU binutils, QEMU system x86-64, and a built
plus-a. QEMU must include its BIOS firmware; QEMU_DATA_DIR can override it.
"""
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def run(*args, cwd=None):
    subprocess.run(args, cwd=cwd, check=True)



def main():
    plus_a = Path(os.environ.get('PLUS_A', ROOT / 'rust/target/release/plus-a')).resolve()
    qemu = os.environ.get('QEMU_SYSTEM_X86_64', 'qemu-system-x86_64')
    with tempfile.TemporaryDirectory(prefix='ascension-boot-') as temp:
        tmp = Path(temp)
        elf = tmp / 'kernel.elf'
        run(str(plus_a), 'build', str(HERE / 'kernel.pa'), '--native', '--freestanding',
            '--load-address', '0x200000', '-o', str(elf))
        run('objcopy', '-O', 'binary', '--only-section=.text', str(elf), str(tmp / 'kernel.bin'))
        run(str(plus_a), 'build', str(HERE / 'kernel.pa'), '--native', '--object',
            '-o', str(tmp / 'kernel64.o'))
        run('objcopy', '-O', 'binary', '--only-section=.text', str(tmp / 'kernel64.o'),
            str(tmp / 'object.bin'))
        if (tmp / 'kernel.bin').read_bytes() != (tmp / 'object.bin').read_bytes():
            raise RuntimeError('freestanding executable and object text differ')
        # The +A object contains x86-64 instructions but this fixture has no
        # relocations. Repackage it as ELF32 so the Multiboot 1 loader can link
        # it with the 32-bit bootstrap; execution still occurs in long mode.
        run('objcopy', '-O', 'elf32-i386', '--rename-section',
            '.text=.kernel,alloc,load,readonly,code,contents', str(tmp / 'kernel64.o'),
            str(tmp / 'kernel32.o'))
        run('as', '--32', str(HERE / 'bootstrap.S'), '-o', str(tmp / 'bootstrap.o'))
        run('ld', '-m', 'elf_i386', '-T', str(HERE / 'bootstrap.ld'),
            '-o', str(tmp / 'boot.elf'), str(tmp / 'bootstrap.o'), str(tmp / 'kernel32.o'))
        socket_path = tmp / 'qmp.sock'
        args = [qemu, '-machine', 'pc,accel=tcg', '-m', '64M', '-display', 'none',
                '-serial', 'none', '-monitor', 'none', '-no-reboot', '-no-shutdown',
                '-qmp', f'unix:{socket_path},server=on,wait=off', '-kernel', str(tmp / 'boot.elf')]
        if os.environ.get('QEMU_DATA_DIR'):
            args += ['-L', os.environ['QEMU_DATA_DIR']]
        proc = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 15
            sock = socket.socket(socket.AF_UNIX)
            while True:
                if proc.poll() is not None:
                    raise RuntimeError('QEMU exited: ' + proc.stderr.read().decode(errors='replace'))
                try:
                    sock.connect(str(socket_path))
                    break
                except (FileNotFoundError, ConnectionRefusedError):
                    if time.monotonic() > deadline:
                        raise RuntimeError('QEMU QMP socket timeout')
                    time.sleep(0.05)
            with sock:
                sock.settimeout(5)
                stream = sock.makefile('rb')
                greeting = json.loads(stream.readline())
                assert 'QMP' in greeting, greeting
                # Read via one buffered stream: avoid losing QMP events in buffering.
                def execute(command, **arguments):
                    sock.sendall((json.dumps({'execute': command, 'arguments': arguments}) + '\n').encode())
                    while True:
                        result = json.loads(stream.readline())
                        if 'error' in result:
                            raise RuntimeError(result['error'])
                        if 'return' in result:
                            return result['return']
                execute('qmp_capabilities')
                while True:
                    execute('stop')
                    registers = execute('human-monitor-command', **{'command-line': 'info registers'})
                    memory = execute('human-monitor-command', **{'command-line': 'xp /1gx 0x300000'})
                    rax = re.search(r'RAX=([0-9a-fA-F]+)', registers)
                    rip = re.search(r'RIP=([0-9a-fA-F]+)', registers)
                    if rax and rip and int(rax.group(1), 16) == 42 and '1122334455667788' in memory.lower():
                        pc = int(rip.group(1), 16)
                        if not 0x200000 <= pc < 0x200000 + (tmp / 'kernel.bin').stat().st_size:
                            raise RuntimeError(f'CPU did not stop in +A kernel: RIP={pc:#x}')
                        op = execute('human-monitor-command', **{'command-line': f'xp /1bx {pc - 1:#x}'})
                        if '0xf4' not in op.lower():
                            raise RuntimeError(f'expected HLT before RIP, got {op}')
                        print(f'PASS: QEMU executed +A kernel; RAX=42, RIP={pc:#x}, memory=0x1122334455667788, HLT')
                        break
                    if time.monotonic() > deadline:
                        raise RuntimeError(f'boot did not reach expected state\n{registers}\n{memory}')
                    execute('cont')
                    time.sleep(0.05)
        finally:
            proc.terminate()
            try:
                proc.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.communicate()


if __name__ == '__main__':
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f'FAIL: {error}', file=sys.stderr)
        sys.exit(1)
