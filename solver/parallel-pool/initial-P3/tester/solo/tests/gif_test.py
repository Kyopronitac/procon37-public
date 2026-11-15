"""Check the actual GIF codec using Pillow as an independent decoder."""
import io
from pathlib import Path
import subprocess
import unittest
from PIL import Image


class GifTests(unittest.TestCase):
    def test_decodes_colors_timing_and_loop(self):
        script = """
import {createGifCodec} from './tester/solo/web/gif-codec.js';
const codec = createGifCodec();
for (const [r,g,b,delay] of [[255,0,0,120],[0,255,0,250],[0,0,255,600]]) {
  const rgba = new Uint8Array(16 * 12 * 4);
  for (let i = 0; i < rgba.length; i += 4) rgba.set([r,g,b,255],i);
  codec.frame({width:16,height:12,delay,buffer:rgba.buffer});
}
process.stdout.write(codec.finish());
"""
        result = subprocess.run(["node", "--input-type=module", "-e", script],
                                cwd=Path(__file__).resolve().parents[3],
                                check=True, capture_output=True, timeout=10)
        image = Image.open(io.BytesIO(result.stdout))
        self.assertEqual(image.size, (16, 12))
        self.assertEqual(image.n_frames, 3)
        self.assertEqual(image.info["loop"], 0)
        for i, (color, delay) in enumerate([((255, 0, 0), 120), ((0, 255, 0), 250), ((0, 0, 255), 600)]):
            image.seek(i)
            self.assertEqual(image.info["duration"], delay)
            self.assertEqual(image.convert("RGB").getpixel((8, 6)), color)


if __name__ == "__main__":
    unittest.main(verbosity=2)
