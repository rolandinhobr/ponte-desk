using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Security.Principal;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;
using Microsoft.Win32;
namespace PonteDesk {
internal static class Program {
    [STAThread]static int Main(string[] args){
        if(args.Contains("--self-test"))return SelfTest.Run();
        if(args.Length==2 && args[0]=="--export-icon"){Artwork.ExportIcon(args[1]);return 0;}
        Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
        if(args.Length==2 && args[0]=="--render-preview"){using(var show=new EventWaitHandle(false,EventResetMode.AutoReset))using(var app=new Desk(new Runtime(Path.Combine(Path.GetTempPath(),"ponte-preview-no-runtime"),"",true),show,true)){app.Export(args[1]);}return 0;}
        string root=Environment.GetEnvironmentVariable("PONTE_DESK_HOME")??Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"PonteDesk");
        bool preview=args.Contains("--preview");
        if(!preview && new WindowsPrincipal(WindowsIdentity.GetCurrent()).IsInRole(WindowsBuiltInRole.Administrator)){MessageBox.Show("Abra Ponte Desk pelo menu Iniciar, como usuário comum. Apenas a instalação e o compartilhamento de cartões pedem administrador.","Ponte Desk");return 1;}
        string suffix=WindowsIdentity.GetCurrent().User.Value+(preview?".Preview":"");bool first;
        using(var mutex=new Mutex(true,@"Local\PonteDesk.UI."+suffix,out first))
        using(var activation=new EventWaitHandle(false,EventResetMode.AutoReset,@"Local\PonteDesk.Show."+suffix)){
            if(!first){activation.Set();return 0;}
            using(var app=new Desk(new Runtime(root,Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"ponte-desk.exe"),preview),activation,args.Contains("--background"))){Application.Run(app);}
            mutex.ReleaseMutex();
        }return 0;
    }
}
internal sealed class DeskButton:Button {
    protected override void OnPaint(PaintEventArgs e){
        e.Graphics.Clear(Artwork.Surface);
        TextRenderer.DrawText(e.Graphics,Text,Font,ClientRectangle,Enabled?Artwork.Accent:Artwork.Muted,TextFormatFlags.HorizontalCenter|TextFormatFlags.VerticalCenter);
        if(Focused&&ShowFocusCues)ControlPaint.DrawFocusRectangle(e.Graphics,new Rectangle(2,2,Width-5,Height-5),Artwork.Accent,Artwork.Surface);
    }
}
internal sealed class PanelWindow:Form { protected override bool ShowWithoutActivation {get{return true;}} }
internal sealed class Desk:ApplicationContext {
    private readonly Runtime runtime;private readonly Storage storage;
    private readonly NotifyIcon tray=new NotifyIcon();private Icon icon;
    private readonly Form panel=new PanelWindow();private readonly FlowLayoutPanel content=new FlowLayoutPanel();
    private readonly System.Windows.Forms.Timer timer=new System.Windows.Forms.Timer();
    private readonly RegisteredWaitHandle activation;
    private Label heading,status,inputStatus,cameraStatus,storageStatus,monitorStatus;
    private Button input,cameraStart,cameraStop,sdStart,sdStop,sdOpen,sdRefresh;
    private ComboBox cameraOrigin,cameras,quality,cardOrigin,cards;
    private CheckBox autoDisplays;private Panel monitorSection;
    private string feedback;private bool busy,polling,exiting;private int ticks;private bool? iconPaused;
    internal Desk(Runtime value,EventWaitHandle show,bool background){
        runtime=value;storage=new Storage(runtime);Build();var handle=panel.Handle;
        activation=ThreadPool.RegisterWaitForSingleObject(show,(state,timed)=>{if(panel.IsHandleCreated)panel.BeginInvoke((Action)Show);},null,-1,false);
        SystemEvents.UserPreferenceChanged+=ThemeChanged;
        tray.Text="Ponte Desk";tray.Visible=true;tray.DoubleClick+=(s,e)=>Show();
        tray.ContextMenuStrip=new ContextMenuStrip();tray.ContextMenuStrip.Opening+=(s,e)=>Menu();
        timer.Interval=1000;timer.Tick+=(s,e)=>Tick();timer.Start();
        try{runtime.StartCore();}catch(Exception e){status.Text=e.Message;}
        Tick();if(!background)Show();
    }
    private Label Label(string text,bool title=false){return new Label{Text=text,AutoSize=true,MaximumSize=new Size(462,0),ForeColor=title?Artwork.Text:Artwork.Muted,Font=new Font("Segoe UI",title?12f:9.5f,title?FontStyle.Bold:FontStyle.Regular),Margin=new Padding(0,2,0,8)};}
    private Button Button(string title,Func<Task> action){var b=new DeskButton{Text=title,AutoSize=true,MinimumSize=new Size(90,32),FlatStyle=FlatStyle.Flat,BackColor=Artwork.Surface,ForeColor=Artwork.Accent,Margin=new Padding(0,2,8,4),Padding=new Padding(6,2,6,2),AccessibleName=title};b.FlatAppearance.BorderSize=0;b.Click+=async(s,e)=>await Run(action);return b;}
    private ComboBox Combo(string name){return new ComboBox{DropDownStyle=ComboBoxStyle.DropDownList,Width=450,AccessibleName=name,Margin=new Padding(0,2,0,9),BackColor=Artwork.Surface,ForeColor=Artwork.Text};}
    private FlowLayoutPanel Row(params Control[] controls){var row=new FlowLayoutPanel{AutoSize=true,WrapContents=true,MaximumSize=new Size(466,0),Margin=Padding.Empty};row.Controls.AddRange(controls);return row;}
    private FlowLayoutPanel Section(string title,params Control[] controls){var section=new FlowLayoutPanel{Width=490,AutoSize=true,FlowDirection=FlowDirection.TopDown,WrapContents=false,BackColor=Artwork.Surface,MinimumSize=new Size(490,0),MaximumSize=new Size(490,0),Padding=new Padding(14),Margin=new Padding(0,0,0,12)};section.Controls.Add(Label(title,true));section.Controls.AddRange(controls);content.Controls.Add(section);return section;}
    private void Build(){
        panel.Text="Ponte Desk";panel.ClientSize=new Size(548,760);panel.MinimumSize=new Size(540,580);panel.StartPosition=FormStartPosition.CenterScreen;panel.AutoScaleMode=AutoScaleMode.Dpi;panel.Font=new Font("Segoe UI",9.5f);panel.BackColor=Artwork.Background;
        panel.FormClosing+=(s,e)=>{if(!exiting){e.Cancel=true;panel.Hide();}};
        panel.HandleCreated+=(s,e)=>Artwork.Chrome(panel);
        var header=new Panel{Dock=DockStyle.Top,Height=106,Padding=new Padding(25,20,20,0),BackColor=Artwork.Background};
        var mark=new PictureBox{Image=Artwork.Mark(48,false,true,false),Location=new Point(25,24),Size=new Size(48,48)};header.Controls.Add(mark);
        heading=Label("Ponte Desk",true);heading.Font=new Font("Segoe UI",19,FontStyle.Bold);heading.Location=new Point(87,17);header.Controls.Add(heading);
        status=Label("Seus computadores, juntos.");status.Location=new Point(89,56);status.MaximumSize=new Size(290,38);header.Controls.Add(status);
        var settings=Button("Ajustes…",OpenSettings);settings.Location=new Point(410,30);settings.Anchor=AnchorStyles.Top|AnchorStyles.Left;header.Controls.Add(settings);
        content.Dock=DockStyle.Fill;content.FlowDirection=FlowDirection.TopDown;content.WrapContents=false;content.AutoScroll=true;content.Padding=new Padding(24,0,16,16);content.BackColor=Artwork.Background;
        panel.Controls.Add(content);panel.Controls.Add(header);
        input=Button("Pausar teclado e mouse",async()=>{await runtime.Bridge(runtime.Paused);RefreshState();});inputStatus=Label("");
        Section("Teclado e mouse",inputStatus,input,Label("Controle local: Ctrl + Alt + Esc"));
        cameraOrigin=Combo("Computador ao qual a webcam está conectada");cameraOrigin.Items.AddRange(new[]{"Conectada a este PC","Conectada ao Mac"});cameraOrigin.SelectedIndex=0;
        cameras=Combo("Webcam USB externa");quality=Combo("Qualidade da webcam");quality.Items.AddRange(new[]{"1080p · 30 fps","720p · 30 fps"});quality.SelectedIndex=0;quality.Width=190;
        cameraOrigin.SelectedIndexChanged+=async(s,e)=>await Run(RefreshCameras);
        cameraStart=Button("Compartilhar webcam",StartCamera);cameraStop=Button("Parar webcam",()=>runtime.StopCamera());cameraStatus=Label("Webcam desligada");
        Section("Webcam externa",cameraOrigin,cameras,quality,Row(cameraStart,cameraStop,Button("Atualizar",RefreshCameras)),cameraStatus,Label("No app de chamada, escolha Ponte Desk Camera."));
        cardOrigin=Combo("Computador ao qual o cartão está conectado");cardOrigin.Items.AddRange(new[]{"Cartão conectado a este PC","Cartão conectado ao Mac"});cardOrigin.SelectedIndex=0;cardOrigin.SelectedIndexChanged+=async(s,e)=>await Run(RefreshCards);
        cards=Combo("Cartão removível para compartilhar somente leitura");storageStatus=Label(storage.Message);
        sdStart=Button("Compartilhar cartão",async()=>{if(storage.Active)await storage.MountCard();else{var choice=cards.SelectedItem as Choice;if(choice==null)throw new InvalidOperationException("Selecione um cartão removível.");await storage.Publish(cardOrigin.SelectedIndex==1,choice.Id);}await storage.Refresh();});
        sdStop=Button("Desmontar e parar",async()=>{await storage.Disconnect();await RefreshCards();});sdOpen=Button("Abrir no destino",()=>storage.Open());sdRefresh=Button("Atualizar",async()=>{await storage.Refresh();await RefreshCards();});
        Section("Cartão SD",cardOrigin,cards,Row(sdStart,sdStop,sdRefresh),sdOpen,storageStatus,Label("Somente leitura. Desmonte e pare antes de retirar o cartão."));
        monitorStatus=Label("");autoDisplays=new CheckBox{Text="Ajustar telas do Mac automaticamente",AutoSize=true,ForeColor=Artwork.Text,Margin=new Padding(0,3,0,8)};autoDisplays.Click+=async(s,e)=>await Run(async()=>{await Patch(new Dictionary<string,object>{{"manage_displays",autoDisplays.Checked}});});
        monitorSection=Section("Monitor compartilhado",autoDisplays,monitorStatus,Button("Restaurar telas do Mac",async()=>{await runtime.Command("settings","restore","--peer",await runtime.Peer());}));
        content.Controls.Add(Row(Button("Reconectar",()=>{runtime.StartCore();return Task.FromResult(0);})));
    }
    internal void Show(){panel.Show();panel.WindowState=FormWindowState.Normal;panel.Activate();if(!runtime.Preview)BeginRefresh();}
    internal void Export(string path){panel.ShowInTaskbar=false;panel.StartPosition=FormStartPosition.Manual;panel.Location=new Point(-20000,-20000);panel.Show();Application.DoEvents();panel.PerformLayout();using(var image=new Bitmap(panel.Width,panel.Height)){panel.DrawToBitmap(image,new Rectangle(0,0,panel.Width,panel.Height));image.Save(path,System.Drawing.Imaging.ImageFormat.Png);}panel.Hide();}
    private async void BeginRefresh(){await Run(async()=>{await RefreshCameras();await storage.Refresh();await RefreshCards();});}
    private async Task Run(Func<Task> action){if(busy)return;busy=true;feedback=null;ControlsState();try{await action();}catch(Exception e){feedback=e.Message;status.Text=e.Message;}finally{busy=false;if(!exiting&&!panel.IsDisposed)RefreshState();}}
    private Dictionary<string,object> Options(){return runtime.Read(File.Exists(Path.Combine(runtime.Root,"peer-preferences.json"))?"peer-preferences.json":"preferences.json");}
    private void RefreshState(){
        var engine=runtime.Read("engine-status.json");var options=Options();bool fresh=DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()-Json.Number(engine,"timestamp_ms")<6500;
        input.Text=runtime.Paused?"Retomar teclado e mouse":"Pausar teclado e mouse";
        inputStatus.Text=runtime.Preview?"Prévia da interface · nenhuma operação real":!fresh?"Motor indisponível. Use Reconectar.":Json.Text(engine,"reason","Aguardando conexão");
        if(!busy)status.Text=feedback??(runtime.Preview?"Prévia da interface":runtime.Paused?"Teclado e mouse em pausa":Json.Flag(engine,"capture_installed")&&fresh?"Pronto para compartilhar":"Controle local protegido");
        if(iconPaused!=runtime.Paused){iconPaused=runtime.Paused;var next=Artwork.Icon(runtime.Paused);tray.Icon=next;if(icon!=null)icon.Dispose();icon=next;panel.Icon=icon;}
        var camera=runtime.Read("webcam-status.json");cameraStatus.Text=Json.Text(camera,"message","Webcam desligada");
        monitorSection.Visible=Json.Flag(options,"shared_monitor");autoDisplays.Checked=Json.Flag(options,"manage_displays");monitorStatus.Text=Json.Text(engine,"reason");
        storageStatus.Text=storage.Message;sdStart.Text=storage.Active?"Montar no destino":"Compartilhar cartão";ControlsState();
    }
    private void ControlsState(){bool free=!busy&&!runtime.Preview;input.Enabled=free;cameraOrigin.Enabled=free;cameras.Enabled=free;cameraStart.Enabled=free&&(Json.Get(Options(),"webcam_enabled")==null||Json.Flag(Options(),"webcam_enabled"));cameraStop.Enabled=free;cardOrigin.Enabled=free&&!storage.Active;cards.Enabled=free&&!storage.Active;sdStart.Enabled=free&&!storage.Pending&&(storage.Active||cards.SelectedItem is Choice);sdStop.Enabled=free&&storage.Active&&!storage.Pending;sdOpen.Enabled=free&&storage.Mounted;sdRefresh.Enabled=free&&!storage.Pending;}
    private async void Tick(){RefreshState();ticks++;if(!runtime.Preview&&!busy&&!polling&&(panel.Visible||storage.Active)&&ticks%3==0){polling=true;try{await storage.Refresh();}catch(Exception e){storage.Message=e.Message;}finally{polling=false;RefreshState();}}}
    private async Task RefreshCameras(){var result=cameraOrigin.SelectedIndex==0?await runtime.Command("webcam","list"):await runtime.Command("webcam","devices","--peer",await runtime.Peer());cameras.Items.Clear();foreach(var d in Json.Items(result,"devices")){string id=Json.Text(d,"id");if(id.Length>0)cameras.Items.Add(new Choice(id,Json.Text(d,"name","Webcam externa")));}if(cameras.Items.Count>0)cameras.SelectedIndex=0;}
    private async Task StartCamera(){if(Json.Get(Options(),"webcam_enabled") is bool&&!Json.Flag(Options(),"webcam_enabled"))throw new InvalidOperationException("Ative a webcam nos Ajustes.");var device=cameras.SelectedItem as Choice;if(device==null)throw new InvalidOperationException("Selecione uma webcam USB externa.");string peer=await runtime.Peer(),q=quality.SelectedIndex==1?"720p":"1080p";await runtime.StopCamera();await Task.Delay(1200);
        if(cameraOrigin.SelectedIndex==1){await runtime.Command("webcam","request","--peer",peer,"--device",device.Id,"--quality",q);return;}
        var args=new[]{"webcam","send","--device",device.Id,"--peer",peer,"--parent",Process.GetCurrentProcess().Id.ToString(),"--quality",q};var info=new ProcessStartInfo(runtime.Core,string.Join(" ",args.Select(Runtime.Quote))){UseShellExecute=false,CreateNoWindow=true};info.EnvironmentVariables["PONTE_DESK_HOME"]=runtime.Root;Process.Start(info).Dispose();
    }
    private async Task RefreshCards(){if(storage.Active)return;var result=await runtime.Command(cardOrigin.SelectedIndex==1?new[]{"storage","list","--remote","--peer",await runtime.Peer()}:new[]{"storage","list"});cards.Items.Clear();foreach(var d in Json.Items(result,"volumes")){if(!Json.Flag(d,"removable"))continue;string id=Json.Text(d,"id");if(id.Length>0)cards.Items.Add(new Choice(id,Json.Text(d,"name","Cartão SD")+" · "+(Json.Number(d,"bytes")/1000000000).ToString("0.#")+" GB"));}if(cards.Items.Count>0)cards.SelectedIndex=0;}
    private async Task Patch(Dictionary<string,object> patch){await runtime.Command("settings","update","--patch",Json.Encode(patch),"--peer",await runtime.Peer());}
    private async Task OpenSettings(){var values=await runtime.Command("settings","show","--peer",await runtime.Peer());using(var form=new Settings(runtime,values)){form.ShowDialog(panel);}RefreshState();}
    private void Menu(){var menu=tray.ContextMenuStrip;menu.Items.Clear();var label=menu.Items.Add(runtime.Paused?"Teclado e mouse em pausa":"Ponte Desk");label.Enabled=false;menu.Items.Add(new ToolStripSeparator());menu.Items.Add("Abrir Ponte Desk",null,(s,e)=>Show());menu.Items.Add(runtime.Paused?"Retomar teclado e mouse":"Pausar teclado e mouse",null,async(s,e)=>await Run(()=>runtime.Bridge(runtime.Paused)));menu.Items.Add("Ajustes…",null,async(s,e)=>{Show();await Run(OpenSettings);});menu.Items.Add("Parar webcam",null,async(s,e)=>await Run(()=>runtime.StopCamera()));menu.Items.Add(new ToolStripSeparator());menu.Items.Add("Encerrar Ponte Desk",null,async(s,e)=>await Quit());}
    private async Task Quit(){if(busy||storage.Pending){Show();status.Text="Conclua a operação ou o pedido do sistema antes de encerrar.";return;}await Run(async()=>{if(storage.Active||Json.Flag(runtime.Read("storage-host.json"),"active")||Json.Flag(runtime.Read("storage-mount.json"),"active")){await storage.Refresh();await storage.Disconnect();}bool cameraFailed=false;try{await runtime.StopCamera();}catch{cameraFailed=true;}if(cameraFailed)await runtime.Command("webcam","stop-local");await runtime.StopCore();exiting=true;tray.Visible=false;panel.Close();ExitThread();});}
    private void ThemeChanged(object sender,UserPreferenceChangedEventArgs e){if(panel.IsHandleCreated)panel.BeginInvoke((Action)(()=>{iconPaused=null;ApplyTheme(panel);Artwork.Chrome(panel);RefreshState();}));}
    private void ApplyTheme(Control c){c.ForeColor=Artwork.Text;c.BackColor=c==panel||c==content?Artwork.Background:Artwork.Surface;foreach(Control child in c.Controls)ApplyTheme(child);}
    protected override void Dispose(bool disposing){if(disposing){timer.Stop();timer.Dispose();activation.Unregister(null);SystemEvents.UserPreferenceChanged-=ThemeChanged;tray.Visible=false;tray.Dispose();if(icon!=null)icon.Dispose();panel.Dispose();}base.Dispose(disposing);}
}
}
