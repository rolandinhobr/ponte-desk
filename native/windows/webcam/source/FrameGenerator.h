#pragma once
#include "FrameBus.h"
class FrameGenerator {
 UINT _width=1280,_height=720;
 FrameBus _bus;
 ULONGLONG _sequence=0,_updated=0;
 std::vector<BYTE> _jpeg,_pixels;
 wil::com_ptr_nothrow<IWICImagingFactory> _factory;
 HRESULT Decode();
public:
 HRESULT EnsureRenderTarget(UINT width,UINT height);
 HRESULT Generate(IMFSample* sample,REFGUID format,IMFSample** output);
};
