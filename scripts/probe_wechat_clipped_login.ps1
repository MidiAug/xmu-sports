# Explicit live diagnostic: only the named login entry, never booking or authorization.
param([ValidateSet('Inspect','Invoke','Message')][string]$Mode='Inspect')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase
$source=@'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Automation;
public class WeChatClippedProbe {
 public delegate bool EnumProc(IntPtr h,IntPtr p);
 [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p,IntPtr l);
 [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc p,IntPtr l);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h,System.Text.StringBuilder s,int n);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h,System.Text.StringBuilder s,int n);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h,uint f);
 [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h,out R r);
 [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr h,ref P p);
 [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 [StructLayout(LayoutKind.Sequential)] public struct R {public int left,top,right,bottom;}
 [StructLayout(LayoutKind.Sequential)] public struct P {public int x,y;}
 static string Class(IntPtr h) {var s=new System.Text.StringBuilder(256);GetClassName(h,s,256);return s.ToString();}
 static string Bounds(R r) {return String.Format("{0},{1},{2},{3}",r.left,r.top,r.right-r.left,r.bottom-r.top);}
 public static string Run(string mode) {
  var targets=new List<IntPtr>();
  EnumWindows((h,p)=>{var s=new System.Text.StringBuilder(256);GetWindowText(h,s,256);if(s.ToString()=="厦大体育") {
   uint pid;GetWindowThreadProcessId(h,out pid);var process=Process.GetProcessById((int)pid);
   if(process.ProcessName=="WeChatAppEx" && process.MainModule.FileName.IndexOf("Tencent",StringComparison.OrdinalIgnoreCase)>=0) targets.Add(h);
  }return true;},IntPtr.Zero);
  if(targets.Count!=1) throw new Exception("TARGET_COUNT_"+targets.Count);
  var rootH=targets[0];uint rootPid;GetWindowThreadProcessId(rootH,out rootPid);
  R before;GetWindowRect(rootH,out before);
  var root=AutomationElement.FromHandle(rootH);
  var nodes=root.FindAll(TreeScope.Descendants,Condition.TrueCondition);
  if(nodes.Count>768) throw new Exception("TREE_TRUNCATED");
  var buttons=new List<AutomationElement>();bool verified=false;
  foreach(AutomationElement e in nodes) {
   if(e.Current.ControlType==ControlType.Document) {object v;if(e.TryGetCurrentPattern(ValuePattern.Pattern,out v))
    verified|=((ValuePattern)v).Current.Value.StartsWith("https://servicewechat.com/wx81a2b2fa90759cb7/");}
   if(e.Current.Name=="立即登录") buttons.Add(e);
  }
  Console.WriteLine("target_verified="+verified+"; login_nodes="+buttons.Count+"; window_before="+Bounds(before));
  var children=new List<IntPtr>();EnumChildWindows(rootH,(h,p)=>{uint pid;GetWindowThreadProcessId(h,out pid);if(pid==rootPid) {children.Add(h);R r;GetWindowRect(h,out r);Console.WriteLine("child_class="+Class(h)+"; bounds="+Bounds(r));}return true;},IntPtr.Zero);
  if(!verified || buttons.Count!=1) throw new Exception("UNVERIFIED_OR_AMBIGUOUS_LOGIN");
  var button=buttons[0];object pattern;
  bool invoke=button.TryGetCurrentPattern(InvokePattern.Pattern,out pattern);
  bool legacy=Array.Exists(button.GetSupportedPatterns(),p=>p.Id==10018);
  var rect=button.Current.BoundingRectangle;
  Console.WriteLine("offscreen="+button.Current.IsOffscreen+"; enabled="+button.Current.IsEnabled+"; invoke="+invoke+"; legacy="+legacy+"; button_bounds="+rect);
  string action="none";
  if(mode=="Invoke" && button.Current.IsEnabled) {
   if(button.TryGetCurrentPattern(InvokePattern.Pattern,out pattern)) {action="Invoke";((InvokePattern)pattern).Invoke();}
  }
  if(mode=="Message" && button.Current.IsEnabled) {
   var receivers=children.FindAll(h=>Class(h)=="Chrome_RenderWidgetHostHWND");
   if(receivers.Count!=1) throw new Exception("RENDERER_NOT_UNIQUE_"+receivers.Count);
   IntPtr receiver=receivers[0];uint pid;GetWindowThreadProcessId(receiver,out pid);
   if(pid!=rootPid || GetAncestor(receiver,2)!=rootH) throw new Exception("RECEIVER_CHANGED");
   var point=new P{x=(int)(rect.Left+rect.Width/2),y=(int)(rect.Top+rect.Height/2)};
   if(!ScreenToClient(receiver,ref point)||point.x<0||point.y<0||point.x>32767||point.y>32767) throw new Exception("COORDINATE_INVALID");
   var pos=(IntPtr)((point.y<<16)|point.x);
   action="PostedRendererMouseMessages";
   if(!PostMessage(receiver,0x200,IntPtr.Zero,pos)||!PostMessage(receiver,0x201,(IntPtr)1,pos)||!PostMessage(receiver,0x202,IntPtr.Zero,pos)) throw new Exception("MESSAGE_OUTCOME_UNKNOWN");
   Console.WriteLine("receiver_client_point="+point.x+","+point.y);
  }
  System.Threading.Thread.Sleep(mode=="Inspect"?0:1800);
  R after;GetWindowRect(rootH,out after);
  Console.WriteLine("attempted_action="+action+"; window_after="+Bounds(after));
  var latest=root.FindAll(TreeScope.Descendants,Condition.TrueCondition);
  var allowed=new HashSet<string>{"立即登录","授权登录","阅读并同意以下协议","场馆预约","统一身份认证","微信授权"};
  foreach(AutomationElement e in latest) if(allowed.Contains(e.Current.Name)) Console.WriteLine("after_control="+e.Current.Name);
  return "diagnostic_finished; no_resize_calls=true; no_global_input=true; no_booking_code=true";
 }
}
'@
Add-Type -TypeDefinition $source -ReferencedAssemblies UIAutomationClient, UIAutomationTypes, WindowsBase
[WeChatClippedProbe]::Run($Mode)
