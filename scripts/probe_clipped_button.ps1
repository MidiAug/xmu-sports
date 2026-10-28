# Isolated WinForms/UIA experiment. Never attaches to WeChat or resizes other windows.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase, System.Windows.Forms, System.Drawing
$source = @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Automation;
using System.Windows.Forms;

public class ClippedButtonProbe : Form {
    protected override bool ShowWithoutActivation { get { return true; } }
    [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    public static List<string> Rows = new List<string>();
    Panel panel; Button button; int clicks; string scenario;
    public ClippedButtonProbe(string name) {
        scenario=name; Text="XMU isolated clipped-button probe";
        StartPosition=FormStartPosition.Manual; Location=new Point(40,40);
        ClientSize=new Size(320,180); FormBorderStyle=FormBorderStyle.FixedToolWindow;
        ShowInTaskbar=false;
        panel=new Panel { Dock=DockStyle.Fill, AutoScroll=name=="scrollable" };
        button=new Button { Text="Probe action", Name="probe-action", Size=new Size(140,36),
            Location=new Point(20,name=="visible"?20:300), Visible=name!="hidden" };
        button.Click+=(s,e)=>Interlocked.Increment(ref clicks);
        panel.Controls.Add(button); Controls.Add(panel);
        Shown+=(s,e)=> {
            IntPtr root=Handle, child=button.Handle;
            var worker=new Thread(()=>Probe(root,child)); worker.IsBackground=true;
            worker.SetApartmentState(ApartmentState.MTA); worker.Start();
        };
    }
    T OnUI<T>(Func<T> f) {return (T)Invoke(f);}
    void Probe(IntPtr root,IntPtr child) {
        try {
            Thread.Sleep(200);
            Size initial=OnUI(()=>Size);
            var el=AutomationElement.FromHandle(root).FindFirst(TreeScope.Descendants,
                new PropertyCondition(AutomationElement.NameProperty,"Probe action"));
            bool found=el!=null, offscreen=found && el.Current.IsOffscreen;
            object pattern=null;
            bool invokable=found && el.TryGetCurrentPattern(InvokePattern.Pattern,out pattern);
            string invokeResult="unsupported";
            int before=Volatile.Read(ref clicks);
            if(invokable) {try {((InvokePattern)pattern).Invoke();invokeResult="returned";}
                catch(Exception e) {invokeResult=e.GetType().Name;}}
            Thread.Sleep(200); int invoked=Volatile.Read(ref clicks)-before;
            int scrollAfterInvoke=OnUI(()=>-panel.AutoScrollPosition.Y);
            // A known native HWND is a stronger capability than an HTML UIA node.
            before=Volatile.Read(ref clicks);
            SendMessage(child,0x0201,(IntPtr)1,(IntPtr)((18<<16)|70));
            SendMessage(child,0x0202,IntPtr.Zero,(IntPtr)((18<<16)|70));
            Thread.Sleep(100); int messaged=Volatile.Read(ref clicks)-before;
            bool scrollItem=found && el.TryGetCurrentPattern(ScrollItemPattern.Pattern,out pattern);
            string scrollResult="unsupported";
            if(scrollItem) {try {((ScrollItemPattern)pattern).ScrollIntoView();scrollResult="returned";}
                catch(Exception e) {scrollResult=e.GetType().Name;}}
            Thread.Sleep(100);
            Size final=OnUI(()=>Size);
            Rows.Add(String.Format("{{\"scenario\":\"{0}\",\"found\":{1},\"offscreen\":{2},\"invoke_supported\":{3},\"invoke_result\":\"{4}\",\"invoke_clicks\":{5},\"native_message_clicks\":{6},\"scroll_item\":\"{7}\",\"size_unchanged\":{8},\"scroll_y\":{9},\"scroll_after_invoke\":{10}}}",
                scenario,found.ToString().ToLower(),offscreen.ToString().ToLower(),invokable.ToString().ToLower(),invokeResult,invoked,messaged,scrollResult,(initial==final).ToString().ToLower(),OnUI(()=>-panel.AutoScrollPosition.Y),scrollAfterInvoke));
        } catch(Exception e) {Rows.Add("{\"scenario\":\""+scenario+"\",\"error\":\""+e.GetType().Name+"\"}");}
        finally {BeginInvoke(new Action(()=>Close()));}
    }
    [STAThread] public static string Run() {
        foreach(string scenario in new[]{"visible","clipped","scrollable","hidden"})
            using(var fixture=new ClippedButtonProbe(scenario)) Application.Run(fixture);
        return "["+String.Join(",",Rows.ToArray())+"]";
    }
}
'@
Add-Type -TypeDefinition $source -ReferencedAssemblies UIAutomationClient, UIAutomationTypes, WindowsBase, System.Windows.Forms, System.Drawing
$result = [ClippedButtonProbe]::Run()
$result
$rows = $result | ConvertFrom-Json
if (($rows | Where-Object { $_.error }) -or ($rows | Where-Object { -not $_.size_unchanged })) { throw 'Probe failed or window changed size' }
if (($rows | Where-Object scenario -eq 'visible').invoke_clicks -ne 1) { throw 'Visible baseline did not invoke' }
if (($rows | Where-Object scenario -eq 'clipped').invoke_clicks -ne 1) { throw 'Clipped native button did not invoke' }
if (($rows | Where-Object scenario -eq 'scrollable').invoke_clicks -ne 1) { throw 'Scrollable native button did not invoke' }
if (($rows | Where-Object scenario -eq 'hidden').found) { throw 'Explicitly hidden button unexpectedly exposed' }
