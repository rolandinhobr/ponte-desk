#define NOMINMAX
#include <windows.h>
#include <oleauto.h>
#include <wincodec.h>
#include <wrl.h>
#include <string>
#include <vector>
#include <chrono>
#include <thread>
#include <cstdio>
using Microsoft::WRL::ComPtr;
using Start=HRESULT(__cdecl*)(const wchar_t*,UINT,UINT,UINT,void**);
using Copy=HRESULT(__cdecl*)(void*,BYTE*,UINT,UINT*);
using Stop=HRESULT(__cdecl*)(void*);
static bool Elevated(){HANDLE token=nullptr;TOKEN_ELEVATION v{};DWORD n=0;bool yes=true;if(OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&token)){if(GetTokenInformation(token,TokenElevation,&v,sizeof(v),&n))yes=v.TokenIsElevated!=0;CloseHandle(token);}return yes;}
static std::string Error(){ComPtr<IErrorInfo> info;if(GetErrorInfo(0,&info)!=S_OK)return {};BSTR text=nullptr;info->GetDescription(&text);if(!text)return {};int n=WideCharToMultiByte(CP_UTF8,0,text,SysStringLen(text),nullptr,0,nullptr,nullptr);std::string utf8(n,0);WideCharToMultiByte(CP_UTF8,0,text,SysStringLen(text),utf8.data(),n,nullptr,nullptr);SysFreeString(text);for(auto& c:utf8)if(c=='"'||c=='\\'||c<' ')c=' ';return utf8;}
int wmain(int argc,wchar_t** argv){
 if(argc!=5)return 2;bool elevated=Elevated();UINT w=wcscmp(argv[3],L"720")==0?1280:1920,h=w==1280?720:1080;HRESULT hr=CoInitializeEx(nullptr,COINIT_MULTITHREADED);bool com=SUCCEEDED(hr);auto module=LoadLibraryExW(argv[2],nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32);void* context=nullptr;HRESULT started=E_FAIL,stopped=S_OK;std::string message;UINT frames=0,actualW=0,actualH=0;UINT minimum=MAXDWORD;double maxVariance=0;
 if(module){auto start=reinterpret_cast<Start>(GetProcAddress(module,"PonteCaptureStart"));auto copy=reinterpret_cast<Copy>(GetProcAddress(module,"PonteCaptureCopy"));auto stop=reinterpret_cast<Stop>(GetProcAddress(module,"PonteCaptureStop"));if(start&&copy&&stop){started=hr=start(argv[4],w,h,30,&context);if(FAILED(hr))message=Error();if(SUCCEEDED(hr)){
  ComPtr<IWICImagingFactory> factory;CoCreateInstance(CLSID_WICImagingFactory,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&factory));std::vector<BYTE> jpeg(2*1024*1024);auto until=std::chrono::steady_clock::now()+std::chrono::seconds(8);
  while(frames<10&&std::chrono::steady_clock::now()<until){UINT length=0;hr=copy(context,jpeg.data(),static_cast<UINT>(jpeg.size()),&length);if(FAILED(hr)){message=Error();break;}if(!length)continue;++frames;minimum=(std::min)(minimum,length);
   ComPtr<IWICStream> stream;ComPtr<IWICBitmapDecoder> decoder;ComPtr<IWICBitmapFrameDecode> frame;ComPtr<IWICFormatConverter> converter;
   hr=factory->CreateStream(&stream);if(SUCCEEDED(hr))hr=stream->InitializeFromMemory(jpeg.data(),length);if(SUCCEEDED(hr))hr=factory->CreateDecoderFromStream(stream.Get(),nullptr,WICDecodeMetadataCacheOnDemand,&decoder);if(SUCCEEDED(hr))hr=decoder->GetFrame(0,&frame);if(SUCCEEDED(hr))hr=frame->GetSize(&actualW,&actualH);if(SUCCEEDED(hr))hr=factory->CreateFormatConverter(&converter);if(SUCCEEDED(hr))hr=converter->Initialize(frame.Get(),GUID_WICPixelFormat24bppBGR,WICBitmapDitherTypeNone,nullptr,0,WICBitmapPaletteTypeCustom);if(SUCCEEDED(hr)&&actualW<=1920&&actualH<=1080){std::vector<BYTE> pixels(actualW*actualH*3);hr=converter->CopyPixels(nullptr,actualW*3,static_cast<UINT>(pixels.size()),pixels.data());if(SUCCEEDED(hr)){double sum=0,sq=0;for(size_t i=0;i<pixels.size();i+=3){double y=(pixels[i]+pixels[i+1]+pixels[i+2])/3.;sum+=y;sq+=y*y;}double count=actualW*actualH;maxVariance=(std::max)(maxVariance,sq/count-(sum/count)*(sum/count));}}if(FAILED(hr)){message=Error();break;}
  }
 }if(context)stopped=stop(context);}}
 else hr=HRESULT_FROM_WIN32(GetLastError());if(module)FreeLibrary(module);if(com)CoUninitialize();bool passed=!elevated&&SUCCEEDED(hr)&&frames==10&&actualW==w&&actualH==h&&maxVariance>.1;char report[4096];int n=snprintf(report,sizeof(report),"{\"passed\":%s,\"elevated\":%s,\"requested_width\":%u,\"requested_height\":%u,\"jpeg_width\":%u,\"jpeg_height\":%u,\"start_hresult\":\"0x%08X\",\"copy_hresult\":\"0x%08X\",\"stop_hresult\":\"0x%08X\",\"frames\":%u,\"minimum_jpeg_bytes\":%u,\"maximum_spatial_variance\":%.3f,\"error_stage\":\"%s\",\"physical_camera\":\"explicit selected external USB only\",\"image_files_created\":false,\"raw_pixels_recorded\":false}\n",passed?"true":"false",elevated?"true":"false",w,h,actualW,actualH,static_cast<UINT>(started),static_cast<UINT>(hr),static_cast<UINT>(stopped),frames,minimum==MAXDWORD?0:minimum,maxVariance,message.c_str());auto file=CreateFileW(argv[1],GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);DWORD written=0;if(file!=INVALID_HANDLE_VALUE){WriteFile(file,report,n,&written,nullptr);CloseHandle(file);}return passed?0:4;
}
