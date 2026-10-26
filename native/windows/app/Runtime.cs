// Ponte Desk. GPL-3.0-or-later. Native Windows shell, no web runtime.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net.Sockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;

namespace PonteDesk {
internal static class Json {
    internal static Dictionary<string,object> Object(object value) { return value as Dictionary<string,object> ?? new Dictionary<string,object>(); }
    internal static object Get(object value,string key) { object item; return Object(value).TryGetValue(key,out item)?item:null; }
    internal static string Text(object value,string key,string fallback="") { return Get(value,key) as string ?? fallback; }
    internal static bool Flag(object value,string key) { return Get(value,key) is bool && (bool)Get(value,key); }
    internal static double Number(object value,string key) { try{return Convert.ToDouble(Get(value,key));}catch{return 0;} }
    internal static object[] Items(object value,string key) { return Get(value,key) as object[] ?? new object[0]; }
    internal static Dictionary<string,object> Read(string path) { try {var f=new FileInfo(path); if(!f.Exists || f.Length>32768)return Object(null);return Parse(File.ReadAllText(path));}catch{return Object(null);} }
    internal static Dictionary<string,object> Parse(string text) { return Object(new JavaScriptSerializer().DeserializeObject(text)); }
    internal static string Encode(object value) { return new JavaScriptSerializer().Serialize(value); }
    internal static void Check(object value) { string e=Text(value,"error"); if(e.Length>0)throw new InvalidOperationException(e); }
}
internal sealed class Runtime {
    internal readonly string Root;
    internal readonly string Core;
    private readonly SemaphoreSlim commands=new SemaphoreSlim(1,1);
    internal bool Preview;
    private string cachedPeer;
    internal Runtime(string root,string core,bool preview) {Root=root;Core=core;Preview=preview;}
    internal Dictionary<string,object> Read(string name) {return Json.Read(Path.Combine(Root,name));}
    // Windows CRT argument escaping, including quotes and trailing backslashes.
    internal static string Quote(string value) {
        var b=new StringBuilder("\"");int slash=0;
        foreach(char c in value){if(c=='\\'){slash++;continue;}if(c=='"'){b.Append('\\',slash*2+1);b.Append(c);}else{b.Append('\\',slash);b.Append(c);}slash=0;}
        b.Append('\\',slash*2);b.Append('"');return b.ToString();
    }
    internal static bool ValidSession(string id,string share) {return id!=null && id.Length==32 && id.All(c=>c>='0'&&c<='9'||c>='a'&&c<='f') && share=="PDSD_"+id;}
    internal async Task<string> Peer() {
        var observer=Read("observer-status.json");string learned=Json.Text(observer,"peer_address");
        string[] peers=Json.Items(Read("observer.json"),"peers").OfType<string>().ToArray();
        if(peers.Contains(learned)){cachedPeer=learned;return learned;}
        if(cachedPeer!=null && peers.Contains(cachedPeer))return cachedPeer;
        foreach(string peer in peers){using(var client=new TcpClient()){try{var task=client.ConnectAsync(peer,4244);if(await Task.WhenAny(task,Task.Delay(400))==task){await task;cachedPeer=peer;return peer;}}catch{}}}
        throw new InvalidOperationException("O outro computador não está acessível. Confira a rede e o Ponte Desk.");
    }
    internal async Task<Dictionary<string,object>> Command(params string[] args) {
        if(Preview)throw new InvalidOperationException("Prévia: nenhuma operação real foi enviada.");
        await commands.WaitAsync();
        try {
            var info=new ProcessStartInfo(Core,string.Join(" ",args.Select(Quote))){UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,StandardOutputEncoding=Encoding.UTF8,StandardErrorEncoding=Encoding.UTF8,WorkingDirectory=Path.GetDirectoryName(Core)};
            info.EnvironmentVariables["PONTE_DESK_HOME"]=Root;
            using(var process=Process.Start(info)){
                Task<string> output=process.StandardOutput.ReadToEndAsync(),error=process.StandardError.ReadToEndAsync();
                await Task.Run(()=>process.WaitForExit());
                string stdout=await output,stderr=await error;
                Dictionary<string,object> result;
                try{result=Json.Parse(stdout.Trim());}catch{try{result=Json.Parse(stderr.Trim().Split('\n').Last());}catch{throw new InvalidOperationException("Não foi possível concluir a operação. Confira a conexão e as autorizações do sistema.");}}
                Json.Check(result);if(process.ExitCode!=0)throw new InvalidOperationException("O sistema não confirmou a operação.");return result;
            }
        } finally {commands.Release();}
    }
    internal Process CoreProcess() {
        try {var state=Read("engine-status.json");int pid=(int)Json.Number(state,"pid");if(pid<2)return null;var p=Process.GetProcessById(pid);if(p.HasExited || !string.Equals(p.MainModule.FileName,Core,StringComparison.OrdinalIgnoreCase)){p.Dispose();return null;}return p;}catch{return null;}
    }
    internal void StartCore() {
        if(Preview)return;using(var existing=CoreProcess()){if(existing!=null)return;}
        if(!File.Exists(Core))throw new InvalidOperationException("Reinstale o Ponte Desk: o motor não foi encontrado.");
        if(!File.Exists(Path.Combine(Root,"observer.json")))throw new InvalidOperationException("Configure o pareamento conforme o guia de instalação.");
        Directory.CreateDirectory(Root);
        var info=new ProcessStartInfo(Core,"--capture-backend windows --emulation-backend windows daemon"){UseShellExecute=false,CreateNoWindow=true,WorkingDirectory=Path.GetDirectoryName(Core)};info.EnvironmentVariables["PONTE_DESK_HOME"]=Root;Process.Start(info).Dispose();
    }
    internal async Task Bridge(bool enabled) {await Command("settings","bridge","--enabled",enabled?"true":"false","--peer",await Peer());}
    internal bool Paused {get{return File.Exists(Path.Combine(Root,"manual-pause"))||Json.Flag(Read("observer-status.json"),"peer_paused");}}
    internal async Task StopCamera() {await Command("webcam","stop-local");await Command("webcam","stop","--peer",await Peer());}
    internal async Task StopCore() {using(var p=CoreProcess()){if(p==null)return;await Command("settings","shutdown","--pid",p.Id.ToString());if(!await Task.Run(()=>p.WaitForExit(10000)))throw new InvalidOperationException("O motor ainda está encerrando. Tente novamente.");}}
}
internal sealed class Choice {
    internal string Id;internal string Name;
    internal Choice(string id,string name){Id=id;Name=name;}
    public override string ToString(){return Name;}
}
}
