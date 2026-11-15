import {GIFEncoder,quantize,applyPalette} from './vendor/gifenc.js';

export function createGifCodec() {
  const gif = GIFEncoder();
  return {
    frame({width,height,delay,buffer}) {
      if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1 || width > 1024 || height > 1200 || buffer.byteLength !== width * height * 4) throw new Error('GIFの画像サイズが不正です。');
      const rgba = new Uint8Array(buffer), palette = quantize(rgba,256);
      gif.writeFrame(applyPalette(rgba,palette),width,height,{palette,delay,repeat:0});
      if (gif.bytesView().length > 64 * 1024 * 1024) throw new Error('GIFが64 MiBを超えました。画像幅を小さくするか、出力範囲を1日にしてください。');
    },
    finish() {gif.finish(); return gif.bytes();}
  };
}
