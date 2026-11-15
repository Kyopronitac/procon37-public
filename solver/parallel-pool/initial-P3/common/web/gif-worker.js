import {createGifCodec} from './gif-codec.js';

const codec = createGifCodec();
self.onmessage = ({data}) => {
  try {
    if (data.type === 'frame') {codec.frame(data); self.postMessage({type:'ready'});}
    else if (data.type === 'finish') {
      const bytes = codec.finish(); self.postMessage({buffer:bytes.buffer},[bytes.buffer]);
    }
  } catch (error) {self.postMessage({error:error.message});}
};
