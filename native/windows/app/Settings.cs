using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using System.Windows.Forms;
namespace PonteDesk {
internal sealed class Settings:Form {
    private readonly Runtime runtime;
    private TextBox macName=new TextBox(),pcName=new TextBox();private ComboBox position=new ComboBox(),windowsMonitor=new ComboBox(),macMonitor=new ComboBox();
    private CheckBox shared,automatic,protectMac,protectPc,webcam;private Label feedback;
    private bool applying;private int macInput,pcInput;private string calibratedWindows,calibratedMac;
    private readonly string[] positions={"left","right","top","bottom"};
    internal Settings(Runtime value,Dictionary<string,object> options){runtime=value;Text="Ajustes do Ponte Desk";Size=new Size(570,750);MinimumSize=new Size(550,600);StartPosition=FormStartPosition.CenterParent;Font=new Font("Segoe UI",10);BackColor=Artwork.Background;ForeColor=Artwork.Text;AutoScaleMode=AutoScaleMode.Dpi;
        var scroll=new FlowLayoutPanel{Dock=DockStyle.Fill,AutoScroll=true,FlowDirection=FlowDirection.TopDown,WrapContents=false,Padding=new Padding(22)};Controls.Add(scroll);
        Add(scroll,"Seus computadores",true);Add(scroll,"Nome do Mac");Field(scroll,macName);macName.Text=Json.Text(options,"local_name","Mac");macName.MaxLength=64;Add(scroll,"Nome do Windows");Field(scroll,pcName);pcName.Text=Json.Text(options,"peer_name","Windows");pcName.MaxLength=64;
        Add(scroll,"Posição do Mac em relação ao Windows");Field(scroll,position);position.DropDownStyle=ComboBoxStyle.DropDownList;position.Items.AddRange(new[]{"À esquerda","À direita","Acima","Abaixo"});position.SelectedIndex=1;
        try{var m=Regex.Match(File.ReadAllText(Path.Combine(runtime.Root,"config.toml")),"(?m)^position\\s*=\\s*\"(left|right|top|bottom)\"");int n=Array.IndexOf(positions,m.Groups[1].Value);if(n>=0)position.SelectedIndex=n;}catch{}
        Add(scroll,"Recursos e proteção",true);protectPc=Check(scroll,"Pausar durante tela cheia no Windows",Json.Flag(options,"protect_windows_fullscreen"));protectMac=Check(scroll,"Proteger tela cheia no Mac",Json.Flag(options,"protect_mac_fullscreen"));webcam=Check(scroll,"Permitir compartilhamento manual da webcam",Json.Flag(options,"webcam_enabled"));
        Add(scroll,"Monitor compartilhado",true);shared=Check(scroll,"Os computadores usam o mesmo monitor",Json.Flag(options,"shared_monitor"));automatic=Check(scroll,"Reorganizar as telas do Mac ao trocar a entrada",Json.Flag(options,"manage_displays"));
        Add(scroll,"A calibração só é necessária para um monitor compartilhado. Troque a entrada física e identifique cada estado.");
        Add(scroll,"Monitor conectado ao Windows");Field(scroll,windowsMonitor);windowsMonitor.DropDownStyle=ComboBoxStyle.DropDownList;
        Add(scroll,"Monitor externo do Mac");Field(scroll,macMonitor);macMonitor.DropDownStyle=ComboBoxStyle.DropDownList;
        macInput=(int)Json.Number(options,"mac_input");pcInput=(int)Json.Number(options,"pc_input");calibratedWindows=Json.Text(options,"windows_monitor");calibratedMac=Json.Text(options,"mac_monitor_uuid");
        if(calibratedWindows.Length>0){windowsMonitor.Items.Add(new Choice(calibratedWindows,"Monitor configurado no Windows"));windowsMonitor.SelectedIndex=0;}
        if(calibratedMac.Length>0){macMonitor.Items.Add(new Choice(calibratedMac,"Monitor configurado no Mac"));macMonitor.SelectedIndex=0;}
        var row=new FlowLayoutPanel{AutoSize=true,Width=500};scroll.Controls.Add(row);ActionButton(row,"Atualizar monitores",RefreshMonitors);ActionButton(row,"Entrada do Mac",()=>Calibrate(true));ActionButton(row,"Entrada do Windows",()=>Calibrate(false));
        feedback=Add(scroll,"As opções são compartilhadas pelos dois computadores.");feedback.MaximumSize=new Size(480,0);
        ActionButton(scroll,"Salvar ajustes",Save);FormClosing+=(s,e)=>{if(applying)e.Cancel=true;};Shown+=(s,e)=>Artwork.Chrome(this);
    }
    private Label Add(Control parent,string title,bool strong=false){var label=new Label{Text=title,AutoSize=true,MaximumSize=new Size(480,0),ForeColor=strong?Artwork.Text:Artwork.Muted,Font=new Font("Segoe UI",strong?12:9.5f,strong?FontStyle.Bold:FontStyle.Regular),Margin=new Padding(0,strong?15:4,0,7)};parent.Controls.Add(label);return label;}
    private void Field(Control parent,Control field){field.Width=480;field.Margin=new Padding(0,0,0,8);field.BackColor=Artwork.Surface;field.ForeColor=Artwork.Text;parent.Controls.Add(field);}
    private CheckBox Check(Control parent,string title,bool value){var check=new CheckBox{Text=title,Checked=value,AutoSize=true,MaximumSize=new Size(480,0),Margin=new Padding(0,2,0,9)};parent.Controls.Add(check);return check;}
    private Button ActionButton(Control parent,string title,Func<Task> action){var b=new DeskButton{Text=title,AutoSize=true,Height=34,FlatStyle=FlatStyle.Flat,BackColor=Artwork.Surface,ForeColor=Artwork.Accent,Margin=new Padding(0,3,8,8)};b.FlatAppearance.BorderSize=0;parent.Controls.Add(b);b.Click+=async(s,e)=>{if(applying)return;applying=true;Enabled=false;try{await action();}catch(Exception error){feedback.Text=error.Message;}finally{applying=false;Enabled=true;}};return b;}
    private async Task<Dictionary<string,object>> WindowsMonitors(){return await runtime.Command("settings","monitors");}
    private async Task RefreshMonitors(){var w=await WindowsMonitors();var m=await runtime.Command("settings","monitors","--peer",await runtime.Peer());LoadChoices(windowsMonitor,w,calibratedWindows);LoadChoices(macMonitor,m,calibratedMac);feedback.Text="Selecione o monitor correto, mude a entrada física e identifique os dois estados.";}
    private void LoadChoices(ComboBox combo,object result,string saved){combo.Items.Clear();foreach(var m in Json.Items(result,"monitors")){string id=Json.Text(m,"id");if(id.Length==0)continue;var c=new Choice(id,Json.Text(m,"name","Monitor externo"));combo.Items.Add(c);if(id==saved)combo.SelectedItem=c;}if(combo.SelectedIndex<0&&combo.Items.Count>0)combo.SelectedIndex=0;}
    private async Task Calibrate(bool mac){var w=windowsMonitor.SelectedItem as Choice;var m=macMonitor.SelectedItem as Choice;if(w==null||m==null)throw new InvalidOperationException("Selecione o monitor nos dois computadores.");var list=await WindowsMonitors();var found=Json.Items(list,"monitors").FirstOrDefault(x=>Json.Text(x,"id")==w.Id);int input=(int)Json.Number(found,"input");if(input<1||input>255)throw new InvalidOperationException("Não foi possível ler a entrada atual do monitor. Confira DDC/CI.");if(calibratedWindows!=w.Id||calibratedMac!=m.Id){macInput=pcInput=0;calibratedWindows=w.Id;calibratedMac=m.Id;}if(mac)macInput=input;else pcInput=input;feedback.Text="Entrada do Mac: "+macInput+" · Windows: "+pcInput+". As duas devem ser diferentes.";}
    private async Task Save(){if(shared.Checked){var w=windowsMonitor.SelectedItem as Choice;var m=macMonitor.SelectedItem as Choice;if(w==null||m==null||w.Id!=calibratedWindows||m.Id!=calibratedMac||macInput<1||pcInput<1||macInput==pcInput)throw new InvalidOperationException("Selecione e calibre duas entradas diferentes para o monitor compartilhado.");}
        var patch=new Dictionary<string,object>{{"local_name",macName.Text.Trim()},{"peer_name",pcName.Text.Trim()},{"protect_windows_fullscreen",protectPc.Checked},{"protect_mac_fullscreen",protectMac.Checked},{"webcam_enabled",webcam.Checked},{"shared_monitor",shared.Checked},{"manage_displays",shared.Checked&&automatic.Checked},{"windows_monitor",calibratedWindows},{"mac_monitor_uuid",calibratedMac},{"mac_input",macInput},{"pc_input",pcInput}};
        string peer=await runtime.Peer();string selected=positions[position.SelectedIndex];string opposite=new[]{"right","left","bottom","top"}[position.SelectedIndex];
        await runtime.Command("settings","layout","--position",opposite,"--peer",peer);await runtime.Command("settings","layout","--position",selected);await runtime.Command("settings","update","--patch",Json.Encode(patch),"--peer",peer);
        if(!webcam.Checked)await runtime.StopCamera();feedback.Text="Ajustes salvos nos dois computadores.";applying=false;Close();
    }
}
}
