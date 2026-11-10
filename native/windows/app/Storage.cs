// Share lifetime is independent of window visibility; unmount must precede stop.
using System;
using System.Collections.Generic;
using System.Threading.Tasks;
namespace PonteDesk {
internal sealed class Storage {
    private readonly Runtime runtime;
    internal string Id,Share,Peer;
    internal bool Remote,Pending;
    internal string Message="Escolha um cartão removível. Acesso somente leitura no outro computador.";
    internal Dictionary<string,object> Host=Json.Object(null),Mount=Json.Object(null);
    internal bool Active {get{return Id!=null;}}
    internal bool Mounted {get{return Json.Text(Mount,"phase")=="mounted"&&Json.Flag(Mount,"active")&&Json.Flag(Mount,"read_only");}}
    internal Storage(Runtime value){runtime=value;}
    internal async Task<Dictionary<string,object>> Route(string verb,bool remote,params string[] extra){var args=new List<string>{"storage",verb};if(remote){args.Add("--remote");args.Add("--peer");args.Add(Peer??await runtime.Peer());}args.AddRange(extra);return await runtime.Command(args.ToArray());}
    internal async Task Refresh() {
        if(Pending)return;Pending=true;
        try {
            Peer=await runtime.Peer();var local=runtime.Read("storage-host.json");var remote=await Route("status",true);
            var other=Json.Object(Json.Get(remote,"host"));
            if(Id==null){var h=(Json.Flag(local,"active")||Json.Text(local,"phase")=="preparing")?local:other;string id=Json.Text(h,"session"),share=Json.Text(h,"share");if(Runtime.ValidSession(id,share)&&(Json.Flag(h,"active")||Json.Text(h,"phase")=="preparing")){Id=id;Share=share;Remote=object.ReferenceEquals(h,other);}}
            if(Id==null)return;
            Host=Remote?other:local;Mount=Remote?runtime.Read("storage-mount.json"):Json.Object(Json.Get(remote,"mount"));
            if(Json.Text(Host,"session")!=Id)throw new InvalidOperationException("A sessão na origem mudou. Atualize antes de continuar.");
            if(Json.Text(Host,"phase")=="error")throw new InvalidOperationException(Json.Text(Host,"error","A origem não confirmou o compartilhamento."));
            if(Json.Text(Mount,"session")==Id&&Json.Text(Mount,"phase")=="error")throw new InvalidOperationException(Json.Text(Mount,"error"));
            Message=Mounted?"Cartão disponível no destino · somente leitura":Json.Text(Host,"phase")=="sharing"?"Origem pronta. Clique em Montar no destino.":"Aguardando autorização na origem…";
        } finally {Pending=false;}
    }
    internal async Task Publish(bool remote,string volume){if(Active)throw new InvalidOperationException("Desmonte a sessão atual primeiro.");Peer=await runtime.Peer();Remote=remote;Pending=true;try{var args=new List<string>{"--id",volume};if(!remote)args.AddRange(new[]{"--peer",Peer,"--parent",System.Diagnostics.Process.GetCurrentProcess().Id.ToString()});var v=await Route("publish",remote,args.ToArray());string id=Json.Text(v,"session"),share=Json.Text(v,"share");if(!Runtime.ValidSession(id,share))throw new InvalidOperationException("Sessão inválida retornada pela origem.");Id=id;Share=share;Message="Autorize o compartilhamento na origem e clique em Montar no destino.";}finally{Pending=false;}}
    private string[] TargetArgs(){var a=new List<string>{"--share",Share,"--session",Id};if(Remote)a.AddRange(new[]{"--peer",Peer});return a.ToArray();}
    internal async Task MountCard(){if(!Active)throw new InvalidOperationException("Selecione um cartão primeiro.");Pending=true;try{var h=await Route("status",Remote);var host=Json.Object(Json.Get(h,"host"));if(Json.Text(host,"session")!=Id||Json.Text(host,"phase")!="sharing"||!Json.Flag(host,"read_only"))throw new InvalidOperationException("Autorize a origem antes de montar o cartão.");await Route("mount",!Remote,TargetArgs());Message="Conectando. Confira o pedido de autenticação no destino.";}finally{Pending=false;}}
    internal async Task Open(){if(!Mounted)throw new InvalidOperationException("O cartão ainda não está montado.");await Route("open",!Remote,"--session",Id);}
    internal async Task Disconnect(){
        if(!Active)return;if(Pending)throw new InvalidOperationException("Há uma operação em andamento. Conclua ou cancele o pedido no sistema.");Pending=true;
        try {
            await Route("unmount",!Remote,TargetArgs());
            // Remote unmount returns an ACK while the system worker is running.
            bool confirmed=false;
            for(int i=0;i<30;i++){var s=await Route("status",!Remote);var m=Json.Object(Json.Get(s,"mount"));if(Json.Text(m,"session")!=Id)throw new InvalidOperationException("A sessão de destino mudou; a origem foi preservada.");if(Json.Text(m,"phase")=="error")throw new InvalidOperationException(Json.Text(m,"error","Feche os arquivos e tente novamente."));if(!Json.Flag(m,"active")&&Json.Text(m,"phase")=="unmounted"){confirmed=true;break;}await Task.Delay(400);}
            if(!confirmed)throw new InvalidOperationException("A desmontagem não foi confirmada. O compartilhamento foi preservado.");
            await Route("stop",Remote,"--session",Id);
            for(int i=0;i<20;i++){var s=await Route("status",Remote);var h=Json.Object(Json.Get(s,"host"));if(Json.Text(h,"session")==Id&&!Json.Flag(h,"active")&&Json.Flag(h,"cleanup_confirmed")){Id=null;Share=null;Host=Json.Object(null);Mount=Json.Object(null);Message="Cartão desconectado com segurança.";return;}await Task.Delay(400);}
            throw new InvalidOperationException("A origem ainda não confirmou a limpeza. Atualize e tente novamente.");
        } finally {Pending=false;}
    }
}
}
