const dgram=require('dgram')
const UDX=require(process.env.UDXPATH)
const raw=dgram.createSocket('udp4')
const got=[]
raw.on('message',(m,r)=>{got.push([Date.now(),m.toString('hex')])})
raw.bind(0,'127.0.0.1',async()=>{
  const port=raw.address().port
  const u=new UDX()
  const sock=u.createSocket(); sock.bind(0,'127.0.0.1')
  const s=u.createStream(5)
  s.on('error',()=>{})
  s.connect(sock,7,port,'127.0.0.1')
  const t0=Date.now()
  // empty MESSAGE
  try{ s.trySend(Buffer.alloc(0)) }catch(e){console.log('trySend err',e.message)}
  // peer announces rwnd=0
  const h=Buffer.alloc(20); h[0]=0xff;h[1]=1;h[2]=0;h[3]=0;h.writeUInt32LE(5,4);h.writeUInt32LE(0,8);h.writeUInt32LE(0,12);h.writeUInt32LE(0,16)
  raw.send(h,sock.address().port,'127.0.0.1')
  setTimeout(()=>{ s.write(Buffer.from('hello')) },100)
  setTimeout(()=>{
    for(const [t,x] of got) console.log(t-t0,'ms',x.length/2,'B',x)
    process.exit(0)
  },7000)
})
