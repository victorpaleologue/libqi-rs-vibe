#pragma once
// Struct types shared by the interop programs, registered with the 2.1 type
// system (annotated tuple signatures such as "(ii)<Point2D,x,y>").

#include <qitype/typeinterface.hpp>

struct Point2D
{
  int x;
  int y;
};
QI_TYPE_STRUCT(Point2D, x, y);

struct TimeStamp
{
  int i;
  int j;
};
QI_TYPE_STRUCT(TimeStamp, i, j);

struct TimeStampedPoint2D
{
  Point2D p;
  TimeStamp t;
};
QI_TYPE_STRUCT(TimeStampedPoint2D, p, t);
