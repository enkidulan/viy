"""Sequential test workload that exercises CPU, memory, I/O, and network.

Each operation runs one after another so resource peaks are clearly
attributable to a single function.
"""

import os
import socket
import time

# --- Constants (so expected values are easy to verify) ---
CPU_BURN_DURATION = 1.5       # seconds per CPU thread
CPU_THREADS = 2

IO_CHUNK_SIZE = 10_000        # bytes per write
IO_CHUNK_COUNT = 50_000        # number of writes
IO_EXPECTED_BYTES = IO_CHUNK_SIZE * IO_CHUNK_COUNT  # 50,000,000 = ~500MB

NET_UDP_PACKETS = 100
NET_UDP_PAYLOAD = 1000        # bytes per packet
NET_EXPECTED_TX = NET_UDP_PACKETS * NET_UDP_PAYLOAD  # 100,000 = ~100KB

MEM_BLOCK_SIZE = 2 * 1024 * 1024  # 2MB per block
MEM_BLOCK_COUNT = 50
MEM_EXPECTED_PEAK = MEM_BLOCK_SIZE * MEM_BLOCK_COUNT  # 100MB

CHILD_PROCESSES = 5


def cpu_burn(duration):
    """Burn CPU for the given duration."""
    end = time.time() + duration
    s = 0
    while time.time() < end:
        for i in range(10000):
            s += i * i


def memory_grow_and_shrink():
    """Allocate memory in stages, then release."""
    blocks = []
    for _ in range(MEM_BLOCK_COUNT):
        blocks.append(bytearray(MEM_BLOCK_SIZE))
        time.sleep(0.02)
    time.sleep(0.2)
    del blocks


def io_write_read():
    """Write a large file, then read it back."""
    path = "/tmp/viy_test_workload"
    written = 0
    with open(path, "wb") as f:
        for _ in range(IO_CHUNK_COUNT):
            f.write(b"x" * IO_CHUNK_SIZE)
            written += IO_CHUNK_SIZE
            f.flush()

    read = 0
    with open(path, "rb") as f:
        while True:
            chunk = f.read(65536)
            if not chunk:
                break
            read += len(chunk)

    os.unlink(path)
    return written, read


def network_activity():
    """Send UDP packets to localhost (loopback)."""
    sent = 0
    for _ in range(NET_UDP_PACKETS):
        try:
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            payload = b"x" * NET_UDP_PAYLOAD
            s.sendto(payload, ("127.0.0.1", 19999))
            sent += len(payload)
            s.close()
        except Exception:
            pass
    return sent


def spawn_children():
    """Fork child processes that do brief CPU work."""
    for _ in range(CHILD_PROCESSES):
        pid = os.fork()
        if pid == 0:
            cpu_burn(0.1)
            os._exit(0)
    for _ in range(CHILD_PROCESSES):
        try:
            os.wait()
        except ChildProcessError:
            break


if __name__ == "__main__":
    print("=" * 60)
    print("Expected metrics:")
    print(f"  I/O write:    {IO_EXPECTED_BYTES:,} bytes ({IO_EXPECTED_BYTES / 1e6:.1f} MB)")
    print(f"  I/O read:     {IO_EXPECTED_BYTES:,} bytes ({IO_EXPECTED_BYTES / 1e6:.1f} MB)")
    print(f"  Network TX:   {NET_EXPECTED_TX:,} bytes ({NET_EXPECTED_TX / 1e3:.1f} KB)")
    print(f"  Memory peak:  ~{MEM_EXPECTED_PEAK / 1e6:.0f} MB")
    print(f"  CPU threads:  {CPU_THREADS} x {CPU_BURN_DURATION}s")
    print(f"  Child procs:  {CHILD_PROCESSES}")
    print("=" * 60)

    cpu_burn(CPU_BURN_DURATION)
    cpu_burn(CPU_BURN_DURATION)
    memory_grow_and_shrink()
    written, read = io_write_read()
    sent = network_activity()
    spawn_children()

    print()
    print("=" * 60)
    print("Actual results from workload:")
    print(f"  I/O written:  {written:,} bytes")
    print(f"  I/O read:     {read:,} bytes")
    print(f"  Network sent: {sent:,} bytes")
    print("=" * 60)
