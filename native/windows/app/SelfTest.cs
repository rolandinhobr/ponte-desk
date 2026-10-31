using System;
using System.IO;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Diagnostics;
using System.Reflection;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;
namespace PonteDesk {
internal static class SelfTest {
    [DllImport("shell32.dll",CharSet=CharSet.Unicode)]static extern IntPtr CommandLineToArgvW(string command,out int count);
    [DllImport("kernel32.dll")]static extern IntPtr LocalFree(IntPtr value);
    internal static int Run(){
        int checks=0;
        try {
            Check(Runtime.Quote("Z:\\")=="\"Z:\\\\\"",ref checks);
            Check(Runtime.Quote("a\"b")=="\"a\\\"b\"",ref checks);
            Check(Runtime.Quote("one two")=="\"one two\"",ref checks);
            foreach(string value in new[]{"Z:\\","","a\"b","duas palavras","á😀\\\\\""}){
                int count;var memory=CommandLineToArgvW("test "+Runtime.Quote(value),out count);
                try{Check(count==2&&Marshal.PtrToStringUni(Marshal.ReadIntPtr(memory,IntPtr.Size))==value,ref checks);}finally{LocalFree(memory);}
            }
            Check(Runtime.ValidSession(new string('a',32),"PDSD_"+new string('a',32)),ref checks);
            Check(!Runtime.ValidSession(new string('A',32),"PDSD_"+new string('A',32)),ref checks);
            Check(!Runtime.ValidSession("../file","PDSD_../file"),ref checks);
            Check(Json.Flag(Json.Parse("{\"active\":true}"),"active"),ref checks);
            Check(!Json.Flag(Json.Parse("{\"active\":\"true\"}"),"active"),ref checks);
            using(var bitmap=Artwork.Mark(64,false,false,false))using(var paused=Artwork.Mark(64,true,false,false)){Check(bitmap.GetPixel(32,32).A>0,ref checks);Check(paused.GetPixel(32,32).A==0,ref checks);}
            string helper=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"PonteDesk.TestWorker.exe");
            if(File.Exists(helper)){
                string root=Path.Combine(Path.GetTempPath(),"ponte-quit-test-"+Guid.NewGuid().ToString("N"));Directory.CreateDirectory(root);
                try {
                    var runtime=new Runtime(root,helper,false);
                    Task pending=runtime.Command();Task queued=runtime.Command();
                    Thread.Sleep(200);Check(!pending.IsCompleted,ref checks);
                    var clock=Stopwatch.StartNew();runtime.ExitNow().GetAwaiter().GetResult();
                    try{Task.WaitAll(new[]{pending,queued},1000);}catch(AggregateException){}
                    Check(clock.ElapsedMilliseconds<1800,ref checks);Check(pending.IsCompleted&&queued.IsCompleted,ref checks);
                    Check(Process.GetProcessesByName("PonteDesk.TestWorker").Length==0,ref checks);
                }finally{Directory.Delete(root,true);}
            }
            Console.WriteLine("Ponte Desk Windows interface: "+checks+" checks passed.");return 0;
        }catch(Exception e){Console.Error.WriteLine(e);return 1;}
    }
    internal static int RunWindowExit(){
        string helper=Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"PonteDesk.TestWorker.exe");
        if(!File.Exists(helper))return 2;
        string root=Path.Combine(Path.GetTempPath(),"ponte-quit-test-"+Guid.NewGuid().ToString("N"));Directory.CreateDirectory(root);
        var watch=Stopwatch.StartNew();
        try {
            using(var show=new EventWaitHandle(false,EventResetMode.AutoReset))
            using(var app=new Desk(new Runtime(root,helper,false),show,true))
            using(var trigger=new System.Windows.Forms.Timer()){
                app.BeginBlockedExitFixture();
                trigger.Interval=250;trigger.Tick+=(s,e)=>{trigger.Stop();app.CloseExitFixture();};trigger.Start();
                Application.Run(app);
            }
            if(watch.ElapsedMilliseconds>2000)throw new Exception("Window close waited on a blocked operation");
            if(Process.GetProcessesByName("PonteDesk.TestWorker").Length>0)throw new Exception("Worker survived GUI close");
            Console.WriteLine("PASS: actual Windows Form.Close exits while a command is blocked");return 0;
        }catch(Exception e){Console.Error.WriteLine(e);return 1;}
        finally{Directory.Delete(root,true);}
    }
    private static void Check(bool value,ref int count){if(!value)throw new Exception("Failed check "+(count+1));count++;}
}
}
