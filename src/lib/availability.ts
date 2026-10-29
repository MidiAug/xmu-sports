export type AvailabilityCell = {court:string;admin_locked:boolean;status:string;reason:string;remaining:number;capacity:number};
export type AvailabilitySlot = {start:string;end:string;cells:AvailabilityCell[]};
export type Availability = {ok:boolean;state:string;message?:string;code?:string;date:string;sport:string;venue:string;cached:boolean;cache_saved?:boolean;account_fingerprint?:string;snapshots?:Record<string,Availability>;fetched_at:string;slots:AvailabilitySlot[];can_choose?:boolean;notice?:string};

const cellsFor = (slot:AvailabilitySlot,court="") => court ? slot.cells.filter(cell=>cell.court===court) : slot.cells;
// Outside booking hours the server still supplies real counts with unavailable status.
const hasQuota = (cell:AvailabilityCell) => ["available", "unavailable"].includes(cell.status)
  && cell.capacity>0 && Number.isFinite(cell.remaining) && cell.remaining>=0;
export function isAdminLocked(slot:AvailabilitySlot,court=""):boolean {
  const cells=cellsFor(slot,court);
  return cells.length>0 && cells.every(cell=>cell.admin_locked);
}

export const courtLabel = (name:string) => /^\d+$/.test(name) ? `${name} 号场地` : name;
export function slotOption(slot:AvailabilitySlot) {
  const locked=slot.cells.some(cell=>cell.admin_locked);
  const unlocked=slot.cells.filter(cell=>!cell.admin_locked);
  const full=unlocked.length>0 && unlocked.every(cell=>hasQuota(cell) && cell.remaining===0);
  return {
    value:`${slot.start}|${slot.end}`, label:`${slot.start}–${slot.end}`,
    badge:isAdminLocked(slot) ? "全部锁定" : locked ? "部分锁定" : full ? "已满" : undefined,
    disabled:isAdminLocked(slot),
  };
}
export function courtOption(cell:AvailabilityCell) {
  return {
    value:cell.court, label:courtLabel(cell.court), disabled:cell.admin_locked,
    badge:cell.admin_locked ? "已锁" : hasQuota(cell) && cell.remaining===0 ? "已满" : undefined,
  };
}

export function availabilityAuthNeeded(result:{code?:string;phase?:string}):boolean {
  return ["NO_ENTRY","INVALID_ENTRY","SESSION_EXPIRED","SESSION_READ_FAILED","NO_SESSION_COOKIE","HTTP_AUTH_UNVERIFIED","LOGIN_CONFIRM_REQUIRED"].includes(result.code ?? "")
    || result.code==="API_REJECTED" && result.phase==="authenticating";
}
