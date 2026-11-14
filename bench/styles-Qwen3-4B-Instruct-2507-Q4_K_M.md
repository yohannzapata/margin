# Rephrase styles: Qwen3-4B-Instruct-2507-Q4_K_M.gguf

## Standard

- **contr-1** `your going to love it, its really good`
  - `You're gonna love it, it's actually really good.` / `This is gonna be awesome, it's really great!`
- **comma-2** `thanks John I really appreciate it`
  - `Thanks, John — I really appreciate it.` / `Thanks, John, I really appreciate that.`
- **period-1** `I finished the report last night I will send it after lunch`
  - `I finished the report last night, gonna send it after lunch.` / `The report is done from last night, sending it after lunch.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - `Hey, I was thinking about what you said yesterday—honestly, I don't think that's going to work because we already tried something similar before and it didn't really go well.` / `I'm still thinking about your suggestion from yesterday, and I'm not sure it'll work since we tried something like that before and it didn't turn out well.`
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Hi Maria, thanks for the update. I checked the numbers and they look good, but the March budget is still missing—can you send it when you get a chance?`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - `I went to the store and they were out of milk, so I got juice instead.`
- **casual-2** `gonna be late, traffic is crazy rn`
  - (rejected: LostToken("gonna"): "Going to be late—traffic is horrible as we speak.")
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was a blast and the lechon was absolutely amazing.` / `Fiesta was so much fun, and the lechon was incredible!`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This phone is way better than my old one and the camera is too good.` / `My new phone is way better than the old one, and the camera is totally great.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `It didn't affect me at all, I'm not sure if it's working.` / `The medicine didn't do anything for me, so I'm not sure it's effective.`
## Formal

- **contr-1** `your going to love it, its really good`
  - (rejected: TooDifferent: "This will be highly enjoyable and of exceptional quality.")
- **comma-2** `thanks John I really appreciate it`
  - `Thank you, John, for your offer—I truly appreciate it.` / `I am grateful, John, for your consideration and support.`
- **period-1** `I finished the report last night I will send it after lunch`
  - `The report was completed last night and will be sent following my lunch break.` / `I have completed the report last night and will transmit it after lunch.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - `Following up on our conversation from yesterday, I am of the opinion that the suggested plan may not succeed, given that a comparable approach was previously tried and did not perform well.`
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Dear Maria, thank you for the update. I have reviewed the numbers and they appear to be accurate; however, the March budget is still missing. Could you please share it when you have the opportunity?`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - `I visited the store and found that milk was unavailable, so I purchased juice as an alternative.` / `Due to the unavailability of milk at the store, I opted for juice instead.`
- **casual-2** `gonna be late, traffic is crazy rn`
  - `I will be arriving later than expected due to significant traffic congestion.` / `I am currently experiencing severe traffic delays and will arrive later than anticipated.`
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was thoroughly enjoyable, and the lechon was exceptionally delicious.` / `The fiesta proved to be a delightful experience, with the lechon standing out as particularly excellent.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This phone is significantly better than my previous one, and its camera performance is exceptional.` / `The current phone offers a substantial improvement over the one I previously used, with particularly outstanding camera capabilities.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `The medication did not have any effect on me, and I am uncertain whether it is effective.`
## Corporate

- **contr-1** `your going to love it, its really good`
  - (rejected: TooDifferent: "The experience will be well-received due to its high standard.")
- **comma-2** `thanks John I really appreciate it`
  - `Thank you, John. I truly appreciate your support.` / `I appreciate your offer, John.`
- **period-1** `I finished the report last night I will send it after lunch`
  - `The report was completed last night and will be shared following the noon lunch break.` / `I completed the report last night and will send it after lunch.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - (rejected: TooDifferent: "After considering your suggestion from yesterday, it appears that the proposed solution may not be effective, as a comparable effort was previously attempted and did not achieve satisfactory outcomes.")
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Hi Maria, thank you for the update. I've reviewed the numbers and they appear accurate, but the March budget is still pending. Could you please share it when available?` / `Hi Maria, thanks for the update. The numbers look favorable, but the March budget is still missing. When you have it, could you send it over?`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - `Milk was unavailable at the store, so juice was purchased as an alternative.` / `Due to the unavailability of milk at the store, juice was selected instead.`
- **casual-2** `gonna be late, traffic is crazy rn`
  - `I will be late due to significant traffic congestion currently affecting my commute.` / `Traffic conditions are severe, resulting in a delayed arrival.`
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was enjoyable, and the lechon was particularly impressive.` / `The event was a success, with the lechon standing out as a highlight.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `The new phone offers significant improvements over the previous model, particularly in camera performance.` / `This device provides notable enhancements compared to the earlier model, with superior camera capabilities.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `The medication did not produce any observable effect on me, and I am uncertain whether it is effective.`
## Simple

- **contr-1** `your going to love it, its really good`
  - `You're gonna love it. It's really good.` / `This is really good. You'll love it.`
- **comma-2** `thanks John I really appreciate it`
  - `Thanks John, I really appreciate it.` / `Thanks, John. I really appreciate it too.`
- **period-1** `I finished the report last night I will send it after lunch`
  - `I finished the report last night. I'll send it after lunch.` / `The report is done from last night. I'm sending it after lunch.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - `Hey, I was thinking about what you said yesterday. I don't think that will work. We tried something like it before, and it didn't go well.` / `I just thought about your comment from yesterday. I don't believe it will work. We already tried something similar, and it didn't turn out well.`
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Hi Maria, thanks for the update. I checked the numbers and they look okay. The March budget is still missing. Can you send it when you have a chance?` / `Hi Maria, thanks for the update. The numbers look fine. The March budget is still missing. Could you send it when you get a chance?`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - (rejected: MeaningFlip("negation"): "I went to the store and found no milk. I picked up juice instead.")
- **casual-2** `gonna be late, traffic is crazy rn`
  - `I'll be late, traffic is really bad right now.` / `Going to be late, traffic is awful right now.`
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was really fun. The lechon was awesome.` / `Fiesta was a lot of fun. Lechon was amazing too.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This phone is much better than my old one and the camera is great.` / `The new phone is better than the old one and the camera works really well.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `The medicine didn't do anything for me. I'm not sure it's working.` / `It didn't affect me at all. I don't think it's working.`
## Creative

- **contr-1** `your going to love it, its really good`
  - `This one’s a game changer, seriously good!`
- **comma-2** `thanks John I really appreciate it`
  - `Hey John, thanks a ton — I really mean it!` / `John, your help means a lot — truly appreciate it!`
- **period-1** `I finished the report last night I will send it after lunch`
  - `I wrapped up the report last night — sending it right after lunch.` / `The report was done last night, and I’ll drop it in after lunch.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - `Hey, I was thinking about what you said yesterday—frankly, I don’t think that’ll work out, since we’ve already tried something like it before and it didn’t go well at all.` / `Honestly, after mulling over what you said yesterday, I’m not sure it’ll work because we tried a similar setup before and it fell apart.`
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Hey Maria, thanks for the update—got a quick glance at the numbers and they're solid, but the March budget's still missing. Could you send it when you've got a moment?` / `Hi Maria, really appreciate the update. The numbers look promising, but the March budget’s still missing. Would you be able to share it when you get a chance?`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - `I hit the store only to find milk vanished—so I grabbed juice instead.`
- **casual-2** `gonna be late, traffic is crazy rn`
  - `Late by the looks of it, traffic’s absolutely crawling rn.`
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was a blast, and the lechon? Absolutely heavenly.` / `Fiesta vibes were off the charts, and that lechon? Pure magic.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This phone is way better than my old one, and the camera's absolutely top-notch.` / `The camera on this phone is next-level, and it's a huge upgrade from my old one.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `It didn’t affect me at all—I’m not convinced it’s doing anything.` / `The medicine had zero impact on me; I’m not sure it’s even working.`
## Academic

- **contr-1** `your going to love it, its really good`
  - (rejected: TooDifferent: "The experience is anticipated to be highly favorable due to its superior quality.")
- **comma-2** `thanks John I really appreciate it`
  - (rejected: TooDifferent: "Thank you, John, for your support, which I have found to be highly appreciated.")
- **period-1** `I finished the report last night I will send it after lunch`
  - `The report was completed last night; it will be transmitted following the midday meal.` / `Completion of the report occurred last night, and it will be submitted after the noon meal.`
- **long-1** `hey i was thinking about what you said yesterday honestly i dont think thats going to work because we already tried something similar before and it didnt really go well`
  - (rejected: TooDifferent: "A reconsideration of your suggestion from yesterday indicates that it may not be viable, given that a comparable effort was previously implemented and did not yield satisfactory results.")
- **long-2** `Hi Maria, thanks for the update. i looked at the numbers and they look good but the budget for march is still missing can you send it when you get a chance`
  - `Dear Maria, thank you for the update. Upon reviewing the figures, the data appears to be satisfactory; however, the March budget remains absent. Could you please provide it at your earliest convenience?` / `Maria, I appreciate the information provided. A review of the figures indicates they are favorable, though the budget for March is currently unavailable. Kindly share it when possible.`
- **run-1** `i went to the store they were out of milk so i got juice instead`
  - `I visited the store, but milk was unavailable; as a result, I purchased juice instead.`
- **casual-2** `gonna be late, traffic is crazy rn`
  - `I will be arriving late due to severe traffic conditions currently in effect.` / `There is a significant traffic congestion currently impacting my ability to arrive on time.`
- **mixed-2** `the fiesta was so much fun, the lechon was amazing`
  - `The fiesta was highly enjoyable, and the lechon was exceptionally excellent.` / `The event proved to be entertaining, with the lechon standing out as a particularly outstanding feature.`
- **ctx-2** `this phone is way better then my old one and the camera is to good`
  - `This telephone demonstrates a significant improvement over the previous model, with the imaging capabilities of the camera being particularly superior.` / `The current device exhibits notable advantages compared to the earlier model, especially in terms of camera performance.`
- **ctx-3** `it didnt effect me at all, I'm not sure its working`
  - `The medication did not produce any observable effect on me; I am uncertain whether it is effective.`

**Standard: 9/10 usable, median 891 ms; Formal: 9/10 usable, median 975 ms; Corporate: 8/10 usable, median 1057 ms; Simple: 9/10 usable, median 845 ms; Creative: 10/10 usable, median 909 ms; Academic: 7/10 usable, median 942 ms**
