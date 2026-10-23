export type AvailabilityCell = {court:string;admin_locked:boolean;status:string;reason:string;remaining:number;capacity:number};
export type AvailabilitySlot = {start:string;end:string;cells:AvailabilityCell[]};
export type Availability = {ok:boolean;state:string;message?:string;code?:string;date:string;sport:string;venue:string;cached:boolean;cache_saved?:boolean;account_fingerprint?:string;snapshots?:Record<string,Availability>;fetched_at:string;slots:AvailabilitySlot[];can_choose?:boolean;notice?:string};

const cellsFor = (slot:AvailabilitySlot,court="") => court ? slot.cells.filter(cell=>cell.court===court) : slot.cells;
// Some sports return numeric IDs, others already include “号场” or a room name.
const courtNames = (cells:AvailabilityCell[]) => cells.every(cell=>/^\d+$/.test(cell.court))
  ? `${cells.map(cell=>cell.court).join("、")} 号场地`
  : cells.map(cell=>/^\d+$/.test(cell.court) ? `${cell.court} 号场地` : cell.court).join("、");
export function isAdminLocked(slot:AvailabilitySlot,court=""):boolean {
  const cells=cellsFor(slot,court);
  return cells.length>0 && cells.every(cell=>cell.admin_locked);
}
export function adminLockLabel(slot:AvailabilitySlot,court=""):string {
  const cells=cellsFor(slot,court);
  if (!cells.length) return "未提供场地状态";
  const locked=cells.filter(cell=>cell.admin_locked);
  if (locked.length===cells.length) return court ? `${courtNames(cells)}管理员锁定` : "全部场地管理员锁定";
  if (locked.length) return `部分管理员锁定（${locked.length}/${cells.length}）：${courtNames(locked)}`;
  return "未发现管理员锁定";
}
export function quotaLabel(slot:AvailabilitySlot,court=""):string {
  const cells=cellsFor(slot,court).filter(cell=>!cell.admin_locked);
  if (!cells.length) return "名额：无未锁定场地数据";
  const known=cells.filter(cell=>cell.status==="available" && cell.remaining>=0);
  const full=known.filter(cell=>cell.remaining===0);
  const remaining=known.reduce((sum,cell)=>sum+cell.remaining,0);
  const fullText=full.length ? `；已满：${courtNames(full)}` : "";
  if (known.length===cells.length && full.length===cells.length) return `名额已满（查询时）${fullText}`;
  if (known.length) return `剩余 ${remaining} 名额（查询时${known.length<cells.length ? "，部分场地未知" : ""}）${fullText}`;
  return "名额状态未知，以学校实时校验为准";
}
export function availabilityClass(slot:AvailabilitySlot,court=""):string {
  return isAdminLocked(slot,court) ? "locked" : "snapshot";
}

export const courtLabel = (name:string) => /^\d+$/.test(name) ? `${name} 号场地` : name;
export function slotOption(slot:AvailabilitySlot) {
  const locked=slot.cells.some(cell=>cell.admin_locked);
  const unlocked=slot.cells.filter(cell=>!cell.admin_locked);
  const full=unlocked.length>0 && unlocked.every(cell=>cell.status==="available" && cell.remaining===0);
  return {
    value:`${slot.start}|${slot.end}`, label:`${slot.start}–${slot.end}`,
    badge:isAdminLocked(slot) ? "全部锁定" : locked ? "部分锁定" : full ? "已满" : undefined,
    disabled:isAdminLocked(slot),
  };
}
export function courtOption(cell:AvailabilityCell) {
  return {
    value:cell.court, label:courtLabel(cell.court), disabled:cell.admin_locked,
    badge:cell.admin_locked ? "已锁" : cell.status==="available" && cell.remaining===0 ? "已满" : undefined,
  };
}

export function availabilityAuthNeeded(result:{code?:string;phase?:string}):boolean {
  return ["NO_ENTRY","INVALID_ENTRY","SESSION_EXPIRED","SESSION_READ_FAILED","NO_SESSION_COOKIE","HTTP_AUTH_UNVERIFIED","LOGIN_CONFIRM_REQUIRED"].includes(result.code ?? "")
    || result.code==="API_REJECTED" && result.phase==="authenticating";
}
