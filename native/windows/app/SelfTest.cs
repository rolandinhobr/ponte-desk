using System;
using System.IO;
using System.Drawing;
using System.Runtime.InteropServices;
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
            Console.WriteLine("Ponte Desk Windows interface: "+checks+" checks passed.");return 0;
        }catch(Exception e){Console.Error.WriteLine(e);return 1;}
    }
    private static void Check(bool value,ref int count){if(!value)throw new Exception("Failed check "+(count+1));count++;}
}
}
