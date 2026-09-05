from pathlib import Path

path = Path("src/portio/virtio_blk_write_readback_fixture.rs")
text = path.read_text()
old = "        let bytes = build_write_readback_guest();\n"
new = "        let bytes = build_write_readback_guest(DescriptorTopology::Direct);\n"
assert text.count(old) == 1, f"direct builder test replacement count={text.count(old)}"
path.write_text(text.replace(old, new))
