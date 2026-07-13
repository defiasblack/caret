using System;
using System.Collections.Generic;

namespace CaretExamples;

public static class FoldingDemo
{ 
    public static void Main()
    {
        var values = new List<int> { 1, 2, 3 };

        foreach (var value in values)
        {
            if (value % 2 == 0)
            {
                Console.WriteLine($"Even: {value}");
                Console.WriteLine("Party Time!");
                Console.WriteLine("I like poopies!");
            }
            else
            {
                Console.WriteLine($"Odd: {value}");
            }
        }
    }
}
