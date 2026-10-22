const path=require('path')
const root=path.join(__dirname,'../../vendor/js')
const m=require(root+'/hyperdht/lib/messages.js')
const c=require(root+'/compact-encoding')
const C=require(root+'/hyperdht/lib/constants.js')
console.log('NS.PLUGIN', C.NS.PLUGIN.toString('hex'), 'CMD', JSON.stringify(C.COMMANDS||{}))
for (const v of [null, Buffer.alloc(0), Buffer.from('aabb','hex')]) {
  const b=c.encode(m.pluginRequest,{plugin:'x',version:1,command:2,value:v})
  console.log('val',v&&v.toString('hex'), b.toString('hex'))
  console.log(' dec', JSON.stringify(c.decode(m.pluginRequest,b)))
}
console.log('no value', c.encode(m.pluginRequest,{plugin:'x',version:1,command:2}).toString('hex'))
