import {readAvailabilityCache,writeAvailabilityCache,type AvailabilityStorage} from "./availability-persistence";
import { availabilityAuthNeeded } from "./availability";
import type { Availability } from "./availability";
export type AvailabilityTarget = {sport:string;venue:string;date:string};
type Failure={value?:Availability;error?:unknown;until:number};
/** Data/request lifetime belongs to the application, not a mounted tab. */
export function createAvailabilityCache(storage?:AvailabilityStorage,now=Date.now) {
  const session={
    cache:new Map<string,{value:Availability;until:number}>(),
    pending:new Map<string,{promise:Promise<Availability>;force:boolean}>(),
    failures:new Map<string,Failure>(),forced:new Map<string,number>(),
    epoch:0,account:undefined as string|undefined,
    automaticFailure:undefined as Failure|undefined,needsLogin:false,persistenceFailed:false,save:()=>{},
  };
  if(storage) {
    const loaded=readAvailabilityCache(storage,now());
    session.persistenceFailed=loaded.failed;
    const data=loaded.data;
    if(data) {
      session.account=data.account;session.needsLogin=data.needsLogin;
      for(const value of data.snapshots) session.cache.set(JSON.stringify([value.sport,value.venue,value.date]),{
        value,until:value.state==="date_not_published" ? Date.parse(value.fetched_at)+30000 : Infinity,
      });
      for(const [key,time] of data.forced) session.forced.set(key,time);
      if(data.retryUntil) session.automaticFailure={error:new Error("查询暂不可用，请稍后重试"),until:data.retryUntil};
    }
    session.save=()=>{
      session.persistenceFailed=!writeAvailabilityCache(storage,{
        account:session.account,needsLogin:session.needsLogin,snapshots:[...session.cache.values()].map(entry=>entry.value),
        forced:[...session.forced],retryUntil:session.automaticFailure?.until,
      },now());
    };
  }
  return session;
}
export const availabilitySessionCache=createAvailabilityCache(typeof window==="undefined" ? undefined : {
  getItem:key=>window.localStorage.getItem(key),setItem:(key,value)=>window.localStorage.setItem(key,value),
});
/** Each view owns cancellation; completed queries still populate the shared cache. */
export function createAvailabilityStore(fetcher:(target:AvailabilityTarget,force:boolean)=>Promise<Availability>, now=Date.now, isCurrent:(target:AvailabilityTarget)=>boolean=()=>true, session=createAvailabilityCache()) {
  const key=(t:AvailabilityTarget)=>JSON.stringify([t.sport,t.venue,t.date]);
  const group=(t:AvailabilityTarget)=>JSON.stringify([t.sport,t.venue]);
  const {cache,pending,failures,forced}=session;
  let disposed=false;
  const peek=(t:AvailabilityTarget):Availability|undefined=>{
    const entry=cache.get(key(t));
    return entry && entry.until>now() ? entry.value : undefined;
  };
  function reset() {session.epoch++;cache.clear();pending.clear();failures.clear();forced.clear();session.automaticFailure=undefined;session.account=undefined;session.needsLogin=false;session.save();}
  async function get(t:AvailabilityTarget,force=false):Promise<Availability> {
    if(disposed) throw new Error("查询已取消");
    const hit=peek(t), id=group(t);
    if(!force && hit) return hit;
    const active=pending.get(id);
    if(active) {
      const result=await active.promise;
      if(disposed) throw new Error("查询已取消");
      if(!result.ok) return result;
      if(force && !active.force && result.cached) return get(t,true);
      return peek(t) ?? get(t,force);
    }
    const other=pending.values().next().value;
    if(other) {
      await other.promise.catch(()=>{});
      if(disposed || !isCurrent(t)) throw new Error("查询已取消");
      return get(t,force);
    }
    if(!isCurrent(t)) throw new Error("查询已取消");
    if(!force && session.automaticFailure && session.automaticFailure.until>now()) {
      if(session.automaticFailure.value) return session.automaticFailure.value;
      throw session.automaticFailure.error;
    }
    const cooling=force && now()-(forced.get(id) ?? -Infinity)<5000;
    if(!force || cooling) {
      const failure=failures.get(id);
      if(failure && failure.until>now()) {
        if(failure.value) return failure.value;
        throw failure.error;
      }
      if(cooling) { if(hit) return hit; throw new Error("请稍后再刷新"); }
    }
    const generation=session.epoch;
    if(force) {forced.set(id,now());session.save();}
    const promise=fetcher(t,force).then(response=>{
      if(generation!==session.epoch) return response;
      if(!response.ok) {
        const failure={value:response,until:now()+30000};
        failures.set(id,failure);
        if(availabilityAuthNeeded(response)) {session.automaticFailure=failure;session.needsLogin=true;}
        session.save();
        return response;
      }
      if(session.account && response.account_fingerprint && session.account!==response.account_fingerprint) {
        session.epoch++;cache.clear();failures.clear();pending.clear();forced.clear();session.needsLogin=false;
      }
      session.account=response.account_fingerprint ?? session.account;
      if(!response.cached) session.needsLogin=false;
      failures.delete(id);session.automaticFailure=undefined;
      const snapshots=response.snapshots ?? {[response.date]:response};
      for(const value of Object.values(snapshots)) {
        if(value.ok && value.sport===t.sport && value.venue===t.venue && value.date) {
          const fetched=Date.parse(value.fetched_at);
          cache.set(key(value),{value,until:value.state==="date_not_published" ? (Number.isFinite(fetched) ? fetched : now())+30000 : Infinity});
        }
      }
      session.save();
      return response;
    }).catch(error=>{
      if(generation===session.epoch) {
        session.automaticFailure={error,until:now()+30000};failures.set(id,session.automaticFailure);session.save();
      }
      throw error;
    }).finally(()=>{if(pending.get(id)?.promise===promise) pending.delete(id);});
    pending.set(id,{promise,force});
    return promise;
  }
  return {peek,get,reset,needsLogin:()=>session.needsLogin,persistenceFailed:()=>session.persistenceFailed,dispose:()=>{disposed=true;}};
}
