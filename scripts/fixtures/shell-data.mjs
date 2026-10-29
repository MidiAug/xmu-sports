import {readFileSync} from "node:fs";
const {sports} = JSON.parse(readFileSync(new URL("../../src/catalog/venues.json",import.meta.url),"utf8"));
export function shellData() {
  const gym=sports.find(s=>s.label.includes("健身"));
  const ping=sports.find(s=>s.label==="乒乓球");
  const rules=[gym,ping].map((sport,index)=>{
    const venue=sport.venues[0],slot=venue.time_slots.find(t=>t.start===(index ? "19:30" : "16:30")) ?? venue.time_slots[0];
    return {id:`123e4567-e89b-42d3-a456-42661417400${index}`,sport:sport.id,venue:venue.id,...slot,weekdays:index?[1,2,3,4,5]:[1,2,3,4,5,6,7],enabled:true,account:"a".repeat(64),first_date:index?"2026-11-02":"2026-11-01"};
  });
  return {plans:{rules,paused:false,attempts:[]},next:rules.map((r,i)=>({id:r.id,date:i?"2026-11-02":"2026-11-01",fire_at:i?"2026-11-01T07:00:00+08:00":"2026-10-31T07:00:00+08:00"})),orders:[
    {order_id:"fixture-order",details_id:"fixture-detail",venue:ping.venues[0].label,sport:ping.label,court:"02",date:"2026-11-01",start:"19:30",end:"21:00",can_cancel:true},
  ]};
}
