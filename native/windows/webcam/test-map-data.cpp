#include "source/pch.h"
#include "source/FrameBus.h"
#include <cstdio>
#include <string>
#include <vector>
int wmain(int argc,wchar_t** argv){
 if(argc!=2)return 2;CoInitializeEx(nullptr,COINIT_MULTITHREADED);wil::com_ptr_nothrow<IWICImagingFactory> factory;CoCreateInstance(CLSID_WICImagingFactory,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&factory));
 std::string json="{\"image_files_created\":false,\"raw_pixels_or_checksums_recorded\":false,\"slots\":[";
 const wchar_t* names[]={PonteMapName,PonteLocalMapName};const wchar_t* locks[]={PonteMutexName,PonteLocalMutexName};
 for(int i=0;i<2;++i){if(i)json+=',';HANDLE write=OpenFileMappingW(FILE_MAP_READ|FILE_MAP_WRITE,FALSE,names[i]);DWORD writeError=write?0:GetLastError();if(write)CloseHandle(write);
 HANDLE map=OpenFileMappingW(FILE_MAP_READ,FALSE,names[i]);DWORD readError=map?0:GetLastError();ULONG active=0,bytes=0;ULONGLONG sequence=0,age=0;UINT width=0,height=0;HRESULT decoded=E_FAIL;double variance=0;
 if(map){auto frame=static_cast<const SharedFrame*>(MapViewOfFile(map,FILE_MAP_READ,0,0,sizeof(SharedFrame)));HANDLE mutex=OpenMutexW(SYNCHRONIZE|MUTEX_MODIFY_STATE,FALSE,locks[i]);
  if(frame&&mutex){auto wait=WaitForSingleObject(mutex,100);if(wait==WAIT_OBJECT_0||wait==WAIT_ABANDONED){active=frame->active;bytes=frame->jpegSize;sequence=frame->sequence;age=GetTickCount64()-frame->updated;std::vector<BYTE> jpeg;if(bytes&&bytes<=PonteMaxJpeg)jpeg.assign(frame->jpeg,frame->jpeg+bytes);ReleaseMutex(mutex);
   if(!jpeg.empty()&&factory){wil::com_ptr_nothrow<IWICStream> stream;decoded=factory->CreateStream(&stream);if(SUCCEEDED(decoded))decoded=stream->InitializeFromMemory(jpeg.data(),static_cast<DWORD>(jpeg.size()));wil::com_ptr_nothrow<IWICBitmapDecoder> decoder;if(SUCCEEDED(decoded))decoded=factory->CreateDecoderFromStream(stream.get(),nullptr,WICDecodeMetadataCacheOnLoad,&decoder);wil::com_ptr_nothrow<IWICBitmapFrameDecode> image;if(SUCCEEDED(decoded))decoded=decoder->GetFrame(0,&image);if(SUCCEEDED(decoded))decoded=image->GetSize(&width,&height);
    if(SUCCEEDED(decoded)&&width<=1920&&height<=1080&&width&&height){wil::com_ptr_nothrow<IWICFormatConverter> converter;decoded=factory->CreateFormatConverter(&converter);if(SUCCEEDED(decoded))decoded=converter->Initialize(image.get(),GUID_WICPixelFormat32bppBGRA,WICBitmapDitherTypeNone,nullptr,0,WICBitmapPaletteTypeCustom);std::vector<BYTE> pixels(static_cast<size_t>(width)*height*4);if(SUCCEEDED(decoded))decoded=converter->CopyPixels(nullptr,width*4,static_cast<UINT>(pixels.size()),pixels.data());if(SUCCEEDED(decoded)){double sum=0,squared=0;for(size_t p=0;p<pixels.size();p+=4){double y=(29*pixels[p]+150*pixels[p+1]+77*pixels[p+2])/256.0;sum+=y;squared+=y*y;}double n=static_cast<double>(width)*height;variance=squared/n-(sum/n)*(sum/n);}}
   }
  }}if(mutex)CloseHandle(mutex);if(frame)UnmapViewOfFile(frame);CloseHandle(map);
 }
 char item[1024];snprintf(item,sizeof(item),"{\"namespace\":\"%s\",\"open_read_error\":%u,\"open_write_error\":%u,\"active\":%u,\"jpeg_size\":%u,\"sequence\":%llu,\"age_ms\":%llu,\"jpeg_width\":%u,\"jpeg_height\":%u,\"decode_hresult\":\"0x%08X\",\"decoded_spatial_variance\":%.3f}",i?"Local":"Global",readError,writeError,active,bytes,sequence,age,width,height,static_cast<UINT>(decoded),variance);json+=item;
 }
 json+="]}";HANDLE report=CreateFileW(argv[1],GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);if(report!=INVALID_HANDLE_VALUE){DWORD written;WriteFile(report,json.data(),static_cast<DWORD>(json.size()),&written,nullptr);CloseHandle(report);}factory.reset();CoUninitialize();return 0;
}
