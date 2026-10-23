import { availabilityAuthNeeded } from "./availability";
import type { Availability } from "./availability";
export type AvailabilityTarget = {sport:string;venue:string;date:string};
/** Scoped to one form/account context. Never polls or starts login/booking. */
export function createAvailabilityStore(fetcher:(target:AvailabilityTarget,force:boolean)=>Promise<Availability>, now=Date.now, isCurrent:(target:AvailabilityTarget)=>boolean=()=>true) {
  const key=(t:AvailabilityTarget)=>JSON.stringify([t.sport,t.venue,t.date]);
  const group=(t:AvailabilityTarget)=>JSON.stringify([t.sport,t.venue]);
  const cache=new Map<string,{value:Availability;until:number}>();
  const pending=new Map<string,{promise:Promise<Availability>;force:boolean}>();
  type Failure={value?:Availability;error?:unknown;until:number};
  const failures=new Map<string,Failure>();
  let automaticFailure:Failure|undefined;
  const forced=new Map<string,number>();
  let epoch=0, disposed=false, account:string|undefined;
  const peek=(t:AvailabilityTarget):Availability|undefined=>{
    const entry=cache.get(key(t));
    return entry && entry.until>now() ? entry.value : undefined;
  };
  function reset() {epoch++;cache.clear();pending.clear();failures.clear();forced.clear();automaticFailure=undefined;account=undefined;}
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
    if(!force && automaticFailure && automaticFailure.until>now()) {
      if(automaticFailure.value) return automaticFailure.value;
      throw automaticFailure.error;
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
    const generation=epoch;
    if(force) forced.set(id,now());
    const promise=fetcher(t,force).then(response=>{
      if(generation!==epoch) return response;
      if(!response.ok) {
        const failure={value:response,until:now()+30000};
        failures.set(id,failure);
        if(availabilityAuthNeeded(response)) automaticFailure=failure;
        return response;
      }
      if(account && response.account_fingerprint && account!==response.account_fingerprint) {
        epoch++;cache.clear();failures.clear();pending.clear();forced.clear();
      }
      account=response.account_fingerprint ?? account;
      failures.delete(id);automaticFailure=undefined;
      const snapshots=response.snapshots ?? {[response.date]:response};
      for(const value of Object.values(snapshots)) {
        if(value.ok && value.sport===t.sport && value.venue===t.venue && value.date) {
          const fetched=Date.parse(value.fetched_at);
          cache.set(key(value),{value,until:value.state==="date_not_published" ? (Number.isFinite(fetched) ? fetched : now())+30000 : Infinity});
        }
      }
      return response;
    }).catch(error=>{
      if(generation===epoch) {
        automaticFailure={error,until:now()+30000};failures.set(id,automaticFailure);
      }
      throw error;
    }).finally(()=>{if(pending.get(id)?.promise===promise) pending.delete(id);});
    pending.set(id,{promise,force});
    return promise;
  }
  return {peek,get,reset,dispose:()=>{reset();disposed=true;}};
}
