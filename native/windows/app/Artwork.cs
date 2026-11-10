using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Runtime.InteropServices;
using Microsoft.Win32;
namespace PonteDesk {
internal static class Artwork {
    [DllImport("user32.dll")]static extern bool DestroyIcon(IntPtr icon);
    [DllImport("dwmapi.dll")]static extern int DwmSetWindowAttribute(IntPtr hwnd,int attribute,ref int value,int size);
    internal static bool Dark {get {try{return (int)Registry.GetValue(@"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize","AppsUseLightTheme",1)==0;}catch{return false;}}}
    internal static Color Background {get{return Dark?Color.FromArgb(25,26,32):Color.FromArgb(247,247,251);}}
    internal static Color Surface {get{return Dark?Color.FromArgb(36,37,45):Color.White;}}
    internal static Color Text {get{return Dark?Color.FromArgb(237,237,244):Color.FromArgb(35,36,47);}}
    internal static Color Muted {get{return Dark?Color.FromArgb(169,171,188):Color.FromArgb(95,98,117);}}
    internal static Color Accent {get{return Dark?Color.FromArgb(173,163,255):Color.FromArgb(97,89,206);}}
    internal static void Chrome(System.Windows.Forms.Form form){int d=Dark?1:0;try{DwmSetWindowAttribute(form.Handle,20,ref d,4);}catch{}}
    internal static Bitmap Mark(int size,bool paused,bool badge,bool tray) {
        var image=new Bitmap(size,size);using(var g=Graphics.FromImage(image)){
            g.SmoothingMode=SmoothingMode.AntiAlias;g.Clear(Color.Transparent);
            if(badge){using(var brush=new SolidBrush(Dark?Color.FromArgb(62,57,87):Color.FromArgb(240,239,252)))g.FillEllipse(brush,0,0,size,size);}
            bool lightTray=false;try{lightTray=(int)Registry.GetValue(@"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize","SystemUsesLightTheme",0)==1;}catch{}
            Color color=tray?(lightTray?Color.FromArgb(40,40,48):Color.White):Accent;
            float scale=size/18f*(badge?.7f:1f);float offset=(size-18*scale)/2;g.TranslateTransform(offset,offset);g.ScaleTransform(scale,scale);
            using(var pen=new Pen(color,1.5f)){pen.StartCap=pen.EndCap=LineCap.Round;pen.LineJoin=LineJoin.Round;
                using(var path=new GraphicsPath()){path.AddLine(7,4.5f,4.5f,4.5f);path.AddBezier(4.5f,4.5f,3.3f,4.5f,2.5f,5.3f,2.5f,6.5f);path.AddLine(2.5f,6.5f,2.5f,11.5f);path.AddBezier(2.5f,11.5f,2.5f,12.7f,3.3f,13.5f,4.5f,13.5f);path.AddLine(4.5f,13.5f,7,13.5f);g.DrawPath(pen,path);}
                using(var path=new GraphicsPath()){path.AddLine(11,4.5f,13.5f,4.5f);path.AddBezier(13.5f,4.5f,14.7f,4.5f,15.5f,5.3f,15.5f,6.5f);path.AddLine(15.5f,6.5f,15.5f,11.5f);path.AddBezier(15.5f,11.5f,15.5f,12.7f,14.7f,13.5f,13.5f,13.5f);path.AddLine(13.5f,13.5f,11,13.5f);g.DrawPath(pen,path);}
                g.DrawLine(pen,6,9,paused?7.5f:12,9);if(paused)g.DrawLine(pen,10.5f,9,12,9);
            }
        }return image;
    }
    internal static Icon Icon(bool paused){using(var bitmap=Mark(32,paused,false,true)){var handle=bitmap.GetHicon();try{return (Icon)System.Drawing.Icon.FromHandle(handle).Clone();}finally{DestroyIcon(handle);}}}
    internal static void ExportIcon(string path){using(var bitmap=Mark(64,false,true,false)){var handle=bitmap.GetHicon();try{using(var icon=System.Drawing.Icon.FromHandle(handle))using(var stream=System.IO.File.Create(path))icon.Save(stream);}finally{DestroyIcon(handle);}}}
}
}
