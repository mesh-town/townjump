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
  s.connect(sock,7,port,'127.0.0.1')
  const t0=Date.now()
  console.log('trySend empty', s.trySend(Buffer.alloc(0)))
  console.log('trySend 3B', s.trySend(Buffer.from('abc')))
  s.setKeepalive(300)
  setTimeout(()=>{
    for(const [t,h] of got) console.log(t-t0,'ms',h.length/2,'B',h)
    process.exit(0)
  },1200)
})
