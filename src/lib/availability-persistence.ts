import type { Availability, AvailabilityCell, AvailabilitySlot } from "./availability";
export type AvailabilityStorage = Pick<Storage,"getItem"|"setItem">;
export type PersistedAvailability = {account?:string;needsLogin:boolean;snapshots:Availability[];forced:[string,number][];retryUntil?:number};
export const AVAILABILITY_CACHE_KEY="xmu-sports.availability.v1";
const text=(v:unknown,max=200):v is string=>typeof v==="string" && v.length<=max;
const object=(v:unknown):v is Record<string,unknown>=>!!v && typeof v==="object" && !Array.isArray(v);
const count=(v:unknown):v is number=>typeof v==="number" && Number.isFinite(v) && v>=0;
/** Only public venue-status fields are stored, never whole RPC replies or credentials. */
function snapshot(raw:unknown,now:number):Availability|undefined {
  if(!object(raw) || raw.ok!==true || !text(raw.sport) || !text(raw.venue) || !text(raw.date,10)
    || !text(raw.fetched_at,50) || !Number.isFinite(Date.parse(raw.fetched_at))
    || !["availability_ready","date_not_published"].includes(String(raw.state))) return;
  const today=new Date(now+8*3600_000).toISOString().slice(0,10);
  const tomorrow=new Date(now+8*3600_000+86400_000).toISOString().slice(0,10);
  if(![today,tomorrow].includes(raw.date)) return;
  if(raw.state==="date_not_published" && Date.parse(raw.fetched_at)+30000<=now) return;
  if(!Array.isArray(raw.slots) || raw.slots.length>256) return;
  const slots:AvailabilitySlot[]=[];
  for(const slot of raw.slots) {
    if(!object(slot) || !text(slot.start,5) || !text(slot.end,5)
      || !/^\d{2}:\d{2}$/.test(slot.start) || !/^\d{2}:\d{2}$/.test(slot.end)
      || !Array.isArray(slot.cells) || slot.cells.length>256) return;
    const cells:AvailabilityCell[]=[];
    for(const cell of slot.cells) {
      if(!object(cell) || !text(cell.court) || typeof cell.admin_locked!=="boolean"
        || !text(cell.status) || !text(cell.reason,2000) || !count(cell.remaining) || !count(cell.capacity)) return;
      cells.push({court:cell.court,admin_locked:cell.admin_locked,status:cell.status,reason:cell.reason,remaining:cell.remaining,capacity:cell.capacity});
    }
    slots.push({start:slot.start,end:slot.end,cells});
  }
  return {ok:true,state:String(raw.state),sport:raw.sport,venue:raw.venue,date:raw.date,fetched_at:raw.fetched_at,slots,cached:true,
    ...(text(raw.account_fingerprint,128) ? {account_fingerprint:raw.account_fingerprint} : {}),
    ...(typeof raw.can_choose==="boolean" ? {can_choose:raw.can_choose} : {}),
    ...(typeof raw.cache_saved==="boolean" ? {cache_saved:raw.cache_saved} : {}),
    ...(text(raw.notice,2000) ? {notice:raw.notice} : {})};
}
function clean(raw:unknown,now:number):PersistedAvailability|undefined {
  if(!object(raw) || raw.version!==1 || !Array.isArray(raw.snapshots)) return;
  const account=text(raw.account,128) ? raw.account : undefined;
  const snapshots=raw.snapshots.slice(-128).map(value=>snapshot(value,now))
    .filter((value):value is Availability=>!!value && (!account || value.account_fingerprint===account));
  const forced:[string,number][]=[];
  if(Array.isArray(raw.forced)) for(const entry of raw.forced.slice(-128)) {
    if(Array.isArray(entry) && text(entry[0],1000) && typeof entry[1]==="number" && entry[1]<=now && entry[1]+5000>now) forced.push([entry[0],entry[1]]);
  }
  const retryUntil=typeof raw.retryUntil==="number" && raw.retryUntil>now && raw.retryUntil<=now+30000 ? raw.retryUntil : undefined;
  return {account,needsLogin:raw.needsLogin===true,snapshots,forced,retryUntil};
}
export function readAvailabilityCache(storage:AvailabilityStorage,now:number):{data?:PersistedAvailability;failed:boolean} {
  try {
    const raw=storage.getItem(AVAILABILITY_CACHE_KEY);
    return {data:raw && raw.length<=2_000_000 ? clean(JSON.parse(raw),now) : undefined,failed:false};
  } catch(error) {return {failed:!(error instanceof SyntaxError)};}
}
export function writeAvailabilityCache(storage:AvailabilityStorage,value:PersistedAvailability,now:number):boolean {
  try {
    const data=clean({version:1,...value},now)!;
    let encoded=JSON.stringify({version:1,...data});
    while(encoded.length>2_000_000 && data.snapshots.length) {
      data.snapshots.shift();encoded=JSON.stringify({version:1,...data});
    }
    storage.setItem(AVAILABILITY_CACHE_KEY,encoded);return true;
  } catch {return false;}
}
